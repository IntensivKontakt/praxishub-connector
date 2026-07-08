//! Z1-PATID-Nachmatch: Cloud-Patienten ohne PVS-Nummer gegen die Z1-`PAT`-Tabelle
//! auflösen und die gefundenen PATIDs an die Cloud zurückmelden.
//!
//! Die Cloud liefert die offenen Patienten (`GET /connector/z1/patients/unmatched`);
//! wir matchen jeden über die bewährte [`resolve_patient`]-Fuzzy-Logik (Name +
//! Geburtsdatum, PLZ als Tiebreaker) und melden nur EINDEUTIGE Treffer
//! (`Resolution::Matched`) zurück (`POST /connector/z1/patients/matched`) — unsichere
//! (`Review`) werden bewusst ausgelassen, um Fehlzuordnungen zu vermeiden.

use std::time::Duration;

use tracing::{debug, info, warn};

use crate::cloud::{CloudClient, PatientMatch};
use crate::config::ConnectorConfig;
use crate::error::Result;
use crate::matching::Resolution;
use crate::z1db::{self, resolve_patient, LoopHandle};

const PAGE: u32 = 500;
const MAX_PAGES_PER_CYCLE: usize = 30; // Sicherheitskappe gegen Endlosschleife

async fn run_cycle(cfg: &ConnectorConfig, cloud: &CloudClient) -> Result<usize> {
    let mut conn = z1db::connect(
        &cfg.z1_db_server,
        &cfg.z1_db_database,
        &cfg.z1_db_user,
        &cfg.z1_db_password,
        cfg.z1_db_trust_cert,
    )
    .await?;

    let mut total = 0usize;
    for _ in 0..MAX_PAGES_PER_CYCLE {
        let batch = cloud.fetch_unmatched_patients(PAGE).await?;
        if batch.is_empty() {
            break;
        }
        let mut matches: Vec<PatientMatch> = Vec::new();
        for p in &batch {
            let zip = Some(p.postal_code.as_str()).filter(|s| !s.is_empty());
            match resolve_patient(&mut conn, &p.last_name, &p.first_name, &p.birth_date, zip).await {
                Ok(Resolution::Matched(patnr)) => matches.push(PatientMatch {
                    cloud_id: p.cloud_id.clone(),
                    patient_id: patnr,
                    matched_by: if zip.is_some() { "name_dob_plz" } else { "name_dob" }.to_string(),
                }),
                Ok(_) => {}                       // Review/NotFound → auslassen (keine Fehlzuordnung)
                Err(e) => debug!(cloud_id=%p.cloud_id, error=%e, "Nachmatch: Lookup fehlgeschlagen"),
            }
        }
        let matched_n = matches.len();
        cloud.report_patient_matches(&matches).await?;
        total += matched_n;
        // Konnte in dieser Seite NICHTS gematcht werden, liefert die nächste Seite dieselben
        // (unveränderten) Patienten → abbrechen, sonst Endlosschleife auf derselben Seite.
        if matched_n == 0 {
            break;
        }
    }
    Ok(total)
}

/// Startet den Nachmatch als eigenständige Schleife. Läuft nur mit Z1-DB-Lesen + Cloud.
/// Zieht beim Start die offenen Patienten durch (mehrere Seiten) und ruht danach lange.
pub fn spawn(cfg: ConnectorConfig) -> LoopHandle {
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        let cloud = match CloudClient::new(&cfg) {
            Ok(c) => c,
            Err(e) => {
                warn!(error=%e, "Nachmatch: Cloud-Client fehlgeschlagen — Schleife beendet");
                return;
            }
        };
        // Stündlich prüfen — der erste Lauf backfillt die Alt-Patienten, danach fällt
        // meist nur Neues an (bzw. nichts).
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        info!("Z1-PATID-Nachmatch gestartet");
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match tokio::time::timeout(Duration::from_secs(600), run_cycle(&cfg, &cloud)).await {
                        Ok(Ok(n)) if n > 0 => info!(gematcht = n, "PATID-Nachmatch-Zyklus"),
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => debug!(error=%e, "PATID-Nachmatch-Zyklus fehlgeschlagen"),
                        Err(_) => warn!("PATID-Nachmatch-Zyklus abgebrochen (Timeout)"),
                    }
                }
                _ = rx.changed() => {
                    if *rx.borrow() { info!("Z1-PATID-Nachmatch gestoppt"); break; }
                }
            }
        }
    });
    LoopHandle::new(tx, join)
}
