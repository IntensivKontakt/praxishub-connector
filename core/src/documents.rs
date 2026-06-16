//! Dokument-Sync (Cloud → PVS-Akte).
//!
//! Holt fertige PDFs (Anamnese/HKP) aus der Praxishub-Cloud
//! (`GET /connector/documents/pending`), legt sie über VDDS-media in die PVS-Akte
//! und quittiert das Ergebnis pro Dokument:
//!   * Erfolg            → `POST /documents/{id}/filed`  (mit der getroffenen Z1-PATID)
//!   * nicht zuzuordnen / Importfehler → `POST /documents/{id}/failed` (mit Grund)
//!
//! „Für genau diesen Patienten": im Push-Modell adressiert VDDS-media den Patienten
//! über die **Z1-PATID**. Fehlt sie (Praxishub kannte sie nicht), kann der Patient
//! nicht eindeutig zugeordnet werden → das Dokument wird als `failed` gemeldet und
//! die Praxis sieht eine „Nicht zugeordnet"-Warnung (statt stiller Endlos-Retrys).
//!
//! ⚠️ **Am Z1-Pilot zu verifizieren** (vgl. [`crate::vdds::media`]): nimmt der PVS
//! ein PDF per media-Push, und wie lautet die exakte CLI-/Trigger-Konvention.

use crate::cloud::{CloudClient, PendingDocument};
use crate::config::ConnectorConfig;
use crate::error::{ConnectorError, Result};
use crate::vdds::media::{self, DocumentKind, ImportRequest, PatientContext};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Was mit einem pending-Dokument geschehen soll — rein aus seinen Daten + der
/// Frage, ob ein PVS-Importprogramm konfiguriert ist (gut unit-testbar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocPlan {
    /// Z1-PATID vorhanden und Importprogramm da → ablegen.
    Import,
    /// Keine Z1-PATID → Patient nicht eindeutig zuzuordnen → als `failed` melden.
    FailNoPatid,
    /// PATID da, aber kein PVS-Importprogramm konfiguriert → pausieren (pending lassen).
    PausedNoProgram,
}

/// Reine Entscheidungslogik (keine Seiteneffekte).
pub fn plan(doc: &PendingDocument, has_program: bool) -> DocPlan {
    if doc.patient_id.trim().is_empty() {
        DocPlan::FailNoPatid
    } else if !has_program {
        DocPlan::PausedNoProgram
    } else {
        DocPlan::Import
    }
}

fn document_kind(kind: &str) -> DocumentKind {
    match kind {
        "hkp" => DocumentKind::Hkp,
        _ => DocumentKind::Anamnese,
    }
}

/// Schreibt das Base64-PDF eines Dokuments in das Austauschverzeichnis.
fn write_pdf(doc: &PendingDocument, dir: &Path) -> Result<PathBuf> {
    let bytes = STANDARD
        .decode(doc.pdf_base64.as_bytes())
        .map_err(|e| ConnectorError::Vdds(format!("PDF-Base64 ungültig: {e}")))?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("praxishub_{}.pdf", doc.id));
    std::fs::write(&path, &bytes)?;
    Ok(path)
}

/// Ergebnis eines Sync-Laufs (für Status/Logging).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DocSyncOutcome {
    pub filed: usize,
    pub failed: usize,
    /// Pausiert, weil (noch) kein PVS-Importprogramm konfiguriert ist.
    pub paused: usize,
}

impl DocSyncOutcome {
    pub fn total(&self) -> usize {
        self.filed + self.failed + self.paused
    }
}

/// Holt alle pending-Dokumente und legt jedes (mit Z1-PATID) über VDDS-media ab.
///
/// Quittiert jedes Dokument einzeln; ein Fehler bei einem Dokument bricht den Lauf
/// nicht ab. Idempotent über die Cloud (bereits abgelegte/fehlgeschlagene Dokumente
/// erscheinen nicht mehr unter `/pending`).
pub async fn sync_pending(cloud: &CloudClient, cfg: &ConnectorConfig) -> Result<DocSyncOutcome> {
    let docs = cloud.fetch_pending_documents().await?;
    let mut out = DocSyncOutcome::default();
    if docs.is_empty() {
        return Ok(out);
    }

    let program = cfg.pvs_import_program_path();
    let dir = cfg.exchange_dir_path();

    for doc in &docs {
        match plan(doc, program.is_some()) {
            DocPlan::FailNoPatid => {
                warn!(id = %doc.id, "Dokument ohne Z1-PATID — nicht eindeutig zuzuordnen");
                if let Err(e) = cloud
                    .mark_document_failed(
                        &doc.id,
                        "Keine Z1-Patientennummer — Patient in Z1 nicht eindeutig zuzuordnen.",
                    )
                    .await
                {
                    warn!(id = %doc.id, error = %e, "Konnte 'failed' nicht melden");
                }
                out.failed += 1;
            }
            DocPlan::PausedNoProgram => {
                out.paused += 1;
            }
            DocPlan::Import => {
                let program = program.as_deref().expect("plan() garantiert Programm");
                if let Err(e) = file_one(cloud, doc, program, &dir).await {
                    warn!(id = %doc.id, error = %e, "PVS-Import fehlgeschlagen");
                    if let Err(e2) = cloud
                        .mark_document_failed(&doc.id, &format!("PVS-Import fehlgeschlagen: {e}"))
                        .await
                    {
                        warn!(id = %doc.id, error = %e2, "Konnte 'failed' nicht melden");
                    }
                    out.failed += 1;
                } else {
                    out.filed += 1;
                }
            }
        }
    }

    info!(
        filed = out.filed,
        failed = out.failed,
        paused = out.paused,
        "Dokument-Sync abgeschlossen"
    );
    Ok(out)
}

/// Legt ein einzelnes Dokument ab und quittiert bei Erfolg mit der Z1-PATID.
async fn file_one(
    cloud: &CloudClient,
    doc: &PendingDocument,
    program: &Path,
    dir: &Path,
) -> Result<()> {
    let pdf_path = write_pdf(doc, dir)?;
    let patient = PatientContext {
        patient_id: doc.patient_id.clone(),
        last_name: doc.last_name.clone(),
        first_name: doc.first_name.clone(),
        birth_date: doc.birth_date.clone(),
    };
    let req = ImportRequest {
        patient: &patient,
        pdf_path: &pdf_path,
        kind: document_kind(&doc.kind),
    };
    media::import_document(program, &req, dir)?;
    info!(id = %doc.id, patid = %doc.patient_id, "Dokument in PVS-Akte abgelegt");
    // Wir adressieren im Push-Modell immer über die mitgelieferte Z1-PATID.
    cloud
        .mark_document_filed(&doc.id, &doc.patient_id, "patient_id")
        .await?;
    let _ = std::fs::remove_file(&pdf_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(patient_id: &str) -> PendingDocument {
        PendingDocument {
            id: "11111111-1111-1111-1111-111111111111".into(),
            kind: "anamnese".into(),
            patient_id: patient_id.into(),
            last_name: "Mustermann".into(),
            first_name: "Erika".into(),
            birth_date: "01.01.1980".into(),
            pdf_base64: STANDARD.encode(b"%PDF-1.4"),
        }
    }

    #[test]
    fn plan_ohne_patid_ist_failnopatid() {
        assert_eq!(plan(&doc(""), true), DocPlan::FailNoPatid);
        assert_eq!(plan(&doc("   "), true), DocPlan::FailNoPatid);
    }

    #[test]
    fn plan_mit_patid_ohne_programm_pausiert() {
        assert_eq!(plan(&doc("4711"), false), DocPlan::PausedNoProgram);
    }

    #[test]
    fn plan_mit_patid_und_programm_importiert() {
        assert_eq!(plan(&doc("4711"), true), DocPlan::Import);
    }

    #[test]
    fn document_kind_mapping() {
        assert!(matches!(document_kind("hkp"), DocumentKind::Hkp));
        assert!(matches!(document_kind("anamnese"), DocumentKind::Anamnese));
        assert!(matches!(document_kind("irgendwas"), DocumentKind::Anamnese));
    }

    #[test]
    fn write_pdf_dekodiert_base64() {
        let dir = std::env::temp_dir().join("praxishub_docsync_test");
        let path = write_pdf(&doc("4711"), &dir).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.4");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_pdf_lehnt_kaputtes_base64_ab() {
        let mut d = doc("4711");
        d.pdf_base64 = "@@nicht-base64@@".into();
        let dir = std::env::temp_dir().join("praxishub_docsync_test");
        assert!(write_pdf(&d, &dir).is_err());
    }
}
