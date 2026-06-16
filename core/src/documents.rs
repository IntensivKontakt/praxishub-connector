//! Dokumenten-Ablage-Orchestrierung: holt anstehende PDFs aus der Cloud und legt
//! sie über VDDS-media in die PVS-Akte — mit der Patienten-Kaskade aus
//! [`crate::vdds::media`] (PATID → Name/Geburtsdatum → Variante A).
//!
//! Zwei Auslöser:
//!   * **Variante B (unbeaufsichtigt):** eine **eigenständige** Schleife
//!     ([`spawn`]) ruft pro Zyklus [`file_pending`] — Dokumente mit bekannter
//!     PATID (~90 %) bzw. eindeutigem Name/Geburtsdatum landen sofort in der
//!     Akte, ohne Klick in Z1. Bewusst **vom KIM-Watcher entkoppelt**: der Push
//!     braucht nur Cloud + importfähiges Z1, kein erreichbares KIM-Postfach.
//!   * **Variante A (Z1-getriggert):** öffnet das Team in Z1 einen Patienten und
//!     ruft unser BVS-Modul auf, übergibt Z1 uns die echte PATID; dann legt
//!     [`file_pending_for_patient`] genau dessen offene Dokumente ab.
//!
//! **Backend-Vertrag offen:** die Routen `documents/pending` + `documents/{id}/filed`
//! müssen serverseitig noch entstehen (siehe [`crate::cloud`]).

use crate::cloud::{CloudClient, PendingDocument};
use crate::config::ConnectorConfig;
use crate::error::Result;
use crate::vdds::ini;
use crate::vdds::media::{self, DocumentKind, FilingOutcome, ImportRequest, PatientContext};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::path::Path;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Laufender Hintergrund-Task des Dokumenten-Push (Variante B).
pub struct DocWatcherHandle {
    shutdown: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<()>,
}

impl DocWatcherHandle {
    /// Signalisiert Stopp und wartet auf das saubere Ende der Schleife.
    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.join.await;
    }
}

/// Startet die unbeaufsichtigte Dokumenten-Ablage als **eigenständige** Schleife.
///
/// Bewusst **entkoppelt vom KIM-Watcher**: Der Anamnese-/HKP-Push in die Z1-Akte
/// braucht nur die Cloud + ein importfähiges Z1 (`MMOINFIMPORT`), aber **kein**
/// erreichbares KIM-Postfach. So läuft die Dokumentenablage weiter, während KIM
/// gerade nicht geht (häufiger Fall) — und umgekehrt.
pub fn spawn(cfg: ConnectorConfig) -> DocWatcherHandle {
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        let period = Duration::from_secs(cfg.doc_poll_seconds.max(10));
        let mut ticker = tokio::time::interval(period);
        info!(period_s = period.as_secs(), "Dokumenten-Push-Schleife gestartet (unabhängig von KIM)");
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match file_pending(&cfg).await {
                        Ok((filed, deferred)) if filed > 0 || deferred > 0 => {
                            debug!(filed, deferred, "Dokumenten-Push-Zyklus");
                        }
                        Ok(_) => {}
                        Err(e) => debug!(error = %e, "Dokumenten-Push-Zyklus fehlgeschlagen"),
                    }
                }
                _ = rx.changed() => {
                    if *rx.borrow() {
                        info!("Dokumenten-Push-Schleife gestoppt");
                        break;
                    }
                }
            }
        }
    });
    DocWatcherHandle { shutdown: tx, join }
}

/// Variante B: alle anstehenden Dokumente unbeaufsichtigt ablegen.
/// Gibt `(abgelegt, zurückgestellt)` zurück. Ist Z1 (noch) nicht
/// importfähig/registriert (`kein MMOINFIMPORT`), passiert nichts (0, 0).
pub async fn file_pending(cfg: &ConnectorConfig) -> Result<(usize, usize)> {
    let Some(import_program) = ini::read_pvs_import_program(&ini::default_ini_path())? else {
        debug!("Kein MMOINFIMPORT in VDDS_MMI.INI — Dokumenten-Push übersprungen");
        return Ok((0, 0));
    };
    let cloud = CloudClient::new(cfg)?;
    let exchange_dir = cfg.exchange_dir_path();

    let docs = cloud.fetch_pending_documents().await?;
    let (mut filed, mut deferred) = (0usize, 0usize);
    for doc in docs {
        match file_one(&import_program, &exchange_dir, &cloud, &doc, None).await {
            Ok(FilingOutcome::Filed) => filed += 1,
            Ok(FilingOutcome::Deferred(reason)) => {
                debug!(id = %doc.id, %reason, "Dokument zurückgestellt (Variante A folgt)");
                deferred += 1;
            }
            Err(e) => warn!(id = %doc.id, error = %e, "Dokument-Ablage fehlgeschlagen"),
        }
    }
    if filed > 0 {
        info!(filed, deferred, "VDDS-media: Dokumente in die PVS-Akte abgelegt");
    }
    Ok((filed, deferred))
}

/// Variante A: legt offene Dokumente ab, die zum gerade in Z1 geöffneten
/// Patienten passen — mit dessen echter, vom PVS übergebener PATID.
/// Match über PATID (falls schon bekannt) oder Name+Geburtsdatum.
pub async fn file_pending_for_patient(
    cfg: &ConnectorConfig,
    patient: &PatientContext,
) -> Result<usize> {
    let Some(import_program) = ini::read_pvs_import_program(&ini::default_ini_path())? else {
        return Ok(0);
    };
    let cloud = CloudClient::new(cfg)?;
    let exchange_dir = cfg.exchange_dir_path();

    let docs = cloud.fetch_pending_documents().await?;
    let mut filed = 0usize;
    for doc in docs {
        if !matches_patient(&doc, patient) {
            continue;
        }
        // Z1-PATID erzwingen — die ist jetzt autoritativ.
        let patid = if patient.has_patid() {
            Some(patient.patient_id.trim())
        } else {
            None
        };
        match file_one(&import_program, &exchange_dir, &cloud, &doc, patid).await {
            Ok(FilingOutcome::Filed) => filed += 1,
            Ok(FilingOutcome::Deferred(reason)) => {
                debug!(id = %doc.id, %reason, "Variante A: weiterhin offen")
            }
            Err(e) => warn!(id = %doc.id, error = %e, "Variante-A-Ablage fehlgeschlagen"),
        }
    }
    if filed > 0 {
        info!(filed, "Variante A: offene Dokumente zum Z1-Patienten abgelegt");
    }
    Ok(filed)
}

/// Legt genau ein Dokument ab und quittiert bei Erfolg ans Backend.
/// `patid_override` überschreibt die (ggf. fehlende) Backend-PATID — für Variante A.
async fn file_one(
    import_program: &Path,
    exchange_dir: &Path,
    cloud: &CloudClient,
    doc: &PendingDocument,
    patid_override: Option<&str>,
) -> Result<FilingOutcome> {
    let pdf_bytes = STANDARD
        .decode(doc.pdf_base64.as_bytes())
        .map_err(|e| crate::error::ConnectorError::Vdds(format!("PDF Base64 ungültig: {e}")))?;
    let pdf_path = exchange_dir.join(format!("praxishub_{}.pdf", sanitize(&doc.id)));
    std::fs::create_dir_all(exchange_dir)?;
    std::fs::write(&pdf_path, &pdf_bytes)?;

    let patient = PatientContext {
        patient_id: patid_override.unwrap_or(doc.patient_id.as_str()).to_string(),
        last_name: doc.last_name.clone(),
        first_name: doc.first_name.clone(),
        birth_date: doc.birth_date.clone(),
    };
    let req = ImportRequest {
        patient: &patient,
        pdf_path: &pdf_path,
        kind: DocumentKind::from_tag(&doc.kind),
    };

    let outcome = media::file_document(import_program, &req, exchange_dir);
    // Temporäres PDF immer wieder aufräumen (Erfolg wie Aufschub).
    let _ = std::fs::remove_file(&pdf_path);
    let outcome = outcome?;

    if matches!(outcome, FilingOutcome::Filed) {
        cloud.ack_document_filed(&doc.id).await?;
    }
    Ok(outcome)
}

/// Passt das Dokument zum geöffneten Patienten? PATID-Gleichheit, sonst
/// Name (case-insensitiv) + Geburtsdatum.
fn matches_patient(doc: &PendingDocument, patient: &PatientContext) -> bool {
    if patient.has_patid()
        && !doc.patient_id.trim().is_empty()
        && doc.patient_id.trim() == patient.patient_id.trim()
    {
        return true;
    }
    !doc.last_name.trim().is_empty()
        && doc.last_name.trim().eq_ignore_ascii_case(patient.last_name.trim())
        && doc.birth_date.trim() == patient.birth_date.trim()
        && !patient.birth_date.trim().is_empty()
}

/// Dateinamens-sichere Variante der Dokument-ID (nur a–z, 0–9, `-`, `_`).
fn sanitize(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if s.is_empty() {
        "doc".into()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: &str, patid: &str, name: &str, dob: &str) -> PendingDocument {
        PendingDocument {
            id: id.into(),
            kind: "anamnese".into(),
            patient_id: patid.into(),
            last_name: name.into(),
            first_name: "Erika".into(),
            birth_date: dob.into(),
            pdf_base64: String::new(),
        }
    }

    fn patient(patid: &str, name: &str, dob: &str) -> PatientContext {
        PatientContext {
            patient_id: patid.into(),
            last_name: name.into(),
            first_name: "Erika".into(),
            birth_date: dob.into(),
        }
    }

    #[test]
    fn match_per_patid() {
        let d = doc("1", "4711", "Mustermann", "01.01.1980");
        assert!(matches_patient(&d, &patient("4711", "Andere", "02.02.1990")));
    }

    #[test]
    fn match_per_name_und_geburtsdatum() {
        let d = doc("1", "", "Mustermann", "01.01.1980");
        assert!(matches_patient(&d, &patient("9999", "mustermann", "01.01.1980")));
    }

    #[test]
    fn kein_match_bei_anderem_geburtsdatum() {
        let d = doc("1", "", "Mustermann", "01.01.1980");
        assert!(!matches_patient(&d, &patient("", "Mustermann", "31.12.1999")));
    }

    #[test]
    fn kein_match_bei_leerem_geburtsdatum() {
        let d = doc("1", "", "Mustermann", "");
        assert!(!matches_patient(&d, &patient("", "Mustermann", "")));
    }

    #[test]
    fn sanitize_entfernt_unsichere_zeichen() {
        assert_eq!(sanitize("a/b 12:3"), "a_b_12_3");
        assert_eq!(sanitize(""), "doc");
    }
}
