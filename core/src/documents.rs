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
            Ok(FilingOutcome::Filed { .. }) => filed += 1,
            Ok(FilingOutcome::Deferred(reason)) => {
                // Offen lassen (Variante A legt es ab, sobald Z1 den Patienten öffnet);
                // NICHT als failed melden, sonst verpasst Variante A das Dokument.
                debug!(id = %doc.id, %reason, "Dokument zurückgestellt (Variante A folgt)");
                deferred += 1;
            }
            Err(e) => {
                // Echter Importfehler → der Cloud melden (Backoff/„nicht zugeordnet").
                warn!(id = %doc.id, error = %e, "Dokument-Ablage fehlgeschlagen");
                let _ = cloud.ack_document_failed(&doc.id, &e.to_string()).await;
            }
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
            Ok(FilingOutcome::Filed { .. }) => filed += 1,
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
    // MMOID = stabile, dateinamen-sichere Dokument-ID. Die Kopie wird unter dem
    // kanonischen Namen (media::mmo_pdf_name) im Austauschordner abgelegt, damit
    // ConVis sie nach dem Push per MMOEXPORT (Pull) genau darüber abholen kann.
    let mmoid = sanitize(&doc.id);
    let pdf_path = exchange_dir.join(media::mmo_pdf_name(&mmoid));
    std::fs::create_dir_all(exchange_dir)?;
    std::fs::write(&pdf_path, &pdf_bytes)?;

    // Effektive Z1-PATID bestimmen — Kaskade:
    //   1. explizit übergeben (Variante A: der gerade in Z1 geöffnete Patient),
    //   2. vom Backend mitgeliefert,
    //   3. Weg A: aus Name+Vorname+Geburtsdatum über die PraxisArchiv-DB auflösen
    //      (für Doctolib-Neupatienten, deren Z1-Nummer erst in der Praxis entsteht,
    //      sobald die Karte „steckt").
    let mut patient_id = patid_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| doc.patient_id.trim().to_string());

    let mut matched_via_lookup = false;
    if patient_id.is_empty() {
        // Der Lookup startet einen kurzlebigen 32-bit-PowerShell-Prozess (COM) und
        // ist damit blockierend → aus dem async-Kontext auslagern.
        let (l, f, d, z) = (
            doc.last_name.clone(),
            doc.first_name.clone(),
            doc.birth_date.clone(),
            doc.zip.clone(),
        );
        let lookup = tokio::task::spawn_blocking(move || {
            crate::patient_lookup::resolve_patient_id(&l, &f, &d, &z)
        })
        .await
        .unwrap_or_else(|e| {
            crate::patient_lookup::PatientLookup::Unavailable(format!("Lookup-Task abgebrochen: {e}"))
        });
        use crate::patient_lookup::PatientLookup;
        match lookup {
            PatientLookup::Found(pid) => {
                info!(id = %doc.id, patid = %pid, "Weg A: PatientenID über PraxisArchiv-DB aufgelöst");
                patient_id = pid;
                matched_via_lookup = true;
            }
            PatientLookup::NotFound => {
                debug!(id = %doc.id, "Weg A: Patient noch nicht in PraxisArchiv — zurückgestellt")
            }
            PatientLookup::Ambiguous => {
                warn!(id = %doc.id, "Weg A: mehrdeutiger Patient (Name/Geburtsdatum) — nicht abgelegt")
            }
            PatientLookup::Unavailable(reason) => {
                debug!(id = %doc.id, %reason, "Weg A: Lookup nicht möglich — zurückgestellt")
            }
        }
    }

    let patient = PatientContext {
        patient_id,
        last_name: doc.last_name.clone(),
        first_name: doc.first_name.clone(),
        birth_date: doc.birth_date.clone(),
    };
    let req = ImportRequest {
        patient: &patient,
        pdf_path: &pdf_path,
        kind: DocumentKind::from_tag(&doc.kind),
    };

    // Die Dokumentkopie bewusst NICHT sofort löschen: ConVis holt sie erst per
    // MMOEXPORT ab (synchron während des Pushs oder später beim Aufruf der Akte)
    // und ist laut VDDS-Spec danach selbst fürs Löschen der Kopie verantwortlich.
    let outcome = media::file_document(import_program, &req, exchange_dir, &mmoid)?;

    if let FilingOutcome::Filed { matched_by } = &outcome {
        // Wurde die PATID per Weg-A-Lookup aufgelöst, das der Cloud so melden
        // (sie bekommt trotzdem die getroffene PATID).
        let effective_matched_by = if matched_via_lookup { "db_lookup" } else { *matched_by };
        // Nur bei „name_dob" (ohne bestätigte PATID) bleibt die gemeldete PATID leer.
        let patid = if effective_matched_by == "name_dob" {
            ""
        } else {
            patient.patient_id.as_str()
        };
        cloud.ack_document_filed(&doc.id, patid, effective_matched_by).await?;
    }
    Ok(outcome)
}

/// Passt das Dokument zum geöffneten Patienten? Zuerst PATID-Gleichheit; sonst
/// ein **starker** Namens-Match (Nachname + Vorname + Geburtsdatum, jeweils
/// normalisiert). Der Vorname ist dabei Pflicht — sonst würden Zwillinge
/// (gleicher Nachname, gleiches Geburtsdatum) vertauscht. Datumsformate
/// (`TT.MM.JJJJ` vs. Z1-`JJJJMMTT`) und Umlaut-Schreibweisen werden über
/// [`crate::matching`] angeglichen.
fn matches_patient(doc: &PendingDocument, patient: &PatientContext) -> bool {
    if patient.has_patid()
        && !doc.patient_id.trim().is_empty()
        && doc.patient_id.trim() == patient.patient_id.trim()
    {
        return true;
    }
    let doc_key = crate::matching::PatientKey::new(&doc.last_name, &doc.first_name, &doc.birth_date);
    let pat_key = crate::matching::PatientKey::new(
        &patient.last_name,
        &patient.first_name,
        &patient.birth_date,
    );
    doc_key.matches(&pat_key)
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
            zip: String::new(),
            email: String::new(),
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
