//! HKP-/EBZ-Tracking über die Z1-DB (ersetzt den KIM-Watcher).
//!
//! Eine eigenständige Schleife pollt read-only die `EBZ`-Tabelle auf neue
//! Kassen-Entscheidungen (`DOKART='3'`), zieht den Voll-HKP (EEBZ0-XML) aus
//! `FILEPOOL` und meldet beides der Cloud. Dedup über einen persistenten
//! Seen-Store (`seen_hkp.json`) — idempotent über PVS-Neustarts.

use crate::cloud::{CloudClient, HkpStatusReport};
use crate::config::ConnectorConfig;
use crate::error::Result;
use crate::paths;
use crate::z1db::{self, LoopHandle, Z1Connection};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Eine gelesene EBZ-Entscheidung (eine Antwortzeile `DOKART='3'`).
#[derive(Debug, Clone)]
struct Decision {
    patnr: String,
    lfdplan: String,
    lfdnr: String,
    erhaltdatum: String,
    zugestellt: String,
    antragsnummer: String,
    planart: String,
}

impl Decision {
    /// Stabiler Dedup-Schlüssel.
    fn key(&self) -> String {
        format!("{}|{}|{}|{}", self.patnr, self.lfdplan, self.lfdnr, self.erhaltdatum)
    }
    /// `"genehmigt"` | `"abgelehnt"` | `"unbekannt"`.
    fn status(&self) -> &'static str {
        match self.zugestellt.trim() {
            "1" => "genehmigt",
            "0" => "abgelehnt",
            _ => "unbekannt",
        }
    }
}

/// Planart-Code → Label (siehe docs/Z1-DATABASE.md §3c).
fn decode_planart(code: &str) -> String {
    match code.trim() {
        "3" => "eHKP",
        "a" => "eHKP (AAV/privat)",
        "4" => "ePAR",
        "7" => "eKBR/KGL",
        "2" => "HKP/ZE",
        other => return format!("Planart {other}"),
    }
    .to_string()
}

/// Persistenter Dedup-Store gemeldeter Entscheidungen.
struct SeenStore {
    set: HashSet<String>,
}

impl SeenStore {
    fn load() -> Result<Self> {
        let path = paths::hkp_seen_store_file()?;
        let set = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => HashSet::new(),
        };
        Ok(Self { set })
    }
    fn contains(&self, key: &str) -> bool {
        self.set.contains(key)
    }
    fn insert(&mut self, key: String) {
        if self.set.insert(key) {
            let _ = self.persist();
        }
    }
    fn persist(&self) -> Result<()> {
        let path = paths::hkp_seen_store_file()?;
        std::fs::write(path, serde_json::to_vec(&self.set)?)?;
        Ok(())
    }
}

/// Liest alle EBZ-Entscheidungen (klein: ~2000 Zeilen), join `ZPLAN` für
/// Antragsnummer + Planart. Dedup/Filter erfolgt beim Aufrufer.
async fn fetch_decisions(conn: &mut Z1Connection) -> Result<Vec<Decision>> {
    let rows = conn
        .rows(
            "SELECT LTRIM(RTRIM(e.PATNR)) AS PATNR, LTRIM(RTRIM(e.LFDPLAN)) AS LFDPLAN, \
                    LTRIM(RTRIM(e.LFDNR)) AS LFDNR, e.ERHALTDATUM AS ERHALT, \
                    ISNULL(e.ZUGESTELLT,'') AS ZUGESTELLT, \
                    LTRIM(RTRIM(ISNULL(z.ANTRAGSNUMMER,''))) AS ANTRAGSNUMMER, \
                    ISNULL(z.PLANART,'') AS PLANART \
             FROM EBZ e \
             LEFT JOIN ZPLAN z ON LTRIM(RTRIM(z.PATNR)) = LTRIM(RTRIM(e.PATNR)) \
                              AND LTRIM(RTRIM(z.LFDPLAN)) = LTRIM(RTRIM(e.LFDPLAN)) \
             WHERE e.DOKART = '3' AND ISNULL(e.ERHALTDATUM,'') <> ''",
            &[],
        )
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(Decision {
            patnr: r.get::<&str, _>("PATNR").unwrap_or("").to_string(),
            lfdplan: r.get::<&str, _>("LFDPLAN").unwrap_or("").to_string(),
            lfdnr: r.get::<&str, _>("LFDNR").unwrap_or("").to_string(),
            erhaltdatum: r.get::<&str, _>("ERHALT").unwrap_or("").to_string(),
            zugestellt: r.get::<&str, _>("ZUGESTELLT").unwrap_or("").to_string(),
            antragsnummer: r.get::<&str, _>("ANTRAGSNUMMER").unwrap_or("").to_string(),
            planart: r.get::<&str, _>("PLANART").unwrap_or("").to_string(),
        });
    }
    Ok(out)
}

/// Holt den Voll-HKP (EEBZ0-XML) aus `FILEPOOL` anhand der Antragsnummer.
async fn fetch_hkp_xml(conn: &mut Z1Connection, antragsnummer: &str) -> Result<Option<Vec<u8>>> {
    if antragsnummer.is_empty() {
        return Ok(None);
    }
    let pattern = format!("EEBZ0_{antragsnummer}%.xml");
    let row = conn
        .one_row(
            "SELECT TOP 1 CAST(FILEDATA AS varbinary(max)) AS DATA FROM FILEPOOL \
             WHERE FILENAME LIKE @P1",
            &[&pattern],
        )
        .await?;
    Ok(row.and_then(|r| r.get::<&[u8], _>("DATA").map(|b| b.to_vec())))
}

/// Ein Poll-Zyklus. Gibt die Anzahl neu gemeldeter Entscheidungen zurück.
async fn poll_once(cfg: &ConnectorConfig, cloud: &CloudClient, seen: &mut SeenStore) -> Result<usize> {
    let mut conn = z1db::connect(
        &cfg.z1_db_server,
        &cfg.z1_db_database,
        &cfg.z1_db_user,
        &cfg.z1_db_password,
        cfg.z1_db_trust_cert,
    )
    .await?;

    let decisions = fetch_decisions(&mut conn).await?;
    let mut reported = 0usize;
    for d in decisions {
        let key = d.key();
        if seen.contains(&key) {
            continue;
        }
        let xml = fetch_hkp_xml(&mut conn, &d.antragsnummer)
            .await
            .unwrap_or(None);
        let report = HkpStatusReport {
            source_key: key.clone(),
            patient_id: d.patnr.clone(),
            plan_no: d.lfdplan.clone(),
            antragsnummer: d.antragsnummer.clone(),
            planart: decode_planart(&d.planart),
            status: d.status().to_string(),
            decided_on: d.erhaltdatum.clone(),
            ehkp_xml_b64: xml.map(|b| STANDARD.encode(b)),
        };
        match cloud.report_hkp_status(&report).await {
            Ok(()) => {
                info!(patnr=%d.patnr, plan=%d.lfdplan, status=d.status(), "HKP-Status gemeldet");
                seen.insert(key); // erst NACH Erfolg → sonst Retry
                reported += 1;
            }
            Err(e) => warn!(patnr=%d.patnr, error=%e, "HKP-Status-Meldung fehlgeschlagen, Retry im nächsten Zyklus"),
        }
    }
    Ok(reported)
}

/// Startet den HKP-Poller als eigenständige Schleife. Läuft nur, wenn Z1-DB-Lesen
/// **und** die Cloud konfiguriert sind.
pub fn spawn(cfg: ConnectorConfig) -> LoopHandle {
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        let cloud = match CloudClient::new(&cfg) {
            Ok(c) => c,
            Err(e) => {
                warn!(error=%e, "HKP-Poller: Cloud-Client fehlgeschlagen — Schleife beendet");
                return;
            }
        };
        let mut seen = SeenStore::load().unwrap_or(SeenStore { set: HashSet::new() });
        let period = Duration::from_secs(cfg.doc_poll_seconds.max(30));
        let mut ticker = tokio::time::interval(period);
        info!(period_s = period.as_secs(), "HKP-Poller (Z1-DB) gestartet");
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match poll_once(&cfg, &cloud, &mut seen).await {
                        Ok(n) if n > 0 => debug!(gemeldet = n, "HKP-Poll-Zyklus"),
                        Ok(_) => {}
                        Err(e) => debug!(error=%e, "HKP-Poll-Zyklus fehlgeschlagen"),
                    }
                }
                _ = rx.changed() => {
                    if *rx.borrow() { info!("HKP-Poller gestoppt"); break; }
                }
            }
        }
    });
    LoopHandle::new(tx, join)
}
