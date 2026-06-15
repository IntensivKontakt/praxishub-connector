//! Dokument-Ablage in die PVS-Akte über VDDS-media.
//!
//! Ablauf (media ist klassisch bild-/datei-zentriert): der BVS schreibt eine
//! Austausch-INI (`VDDS_MMO.INI`) mit Patientenkontext + zu importierender Datei
//! und ruft das vom PVS in `VDDS_MMI.INI` registrierte Import-Programm auf.
//!
//! ⚠️ **Am Z1-Pilot zu verifizieren** (PRA-15, Prüfpunkte 2–3):
//!   * Nimmt der PVS ein **PDF** in die **Dokumentenablage** (nicht nur Bild-Roh-Import)?
//!   * Trigger: PVS-Button vs. Archiv-Pull (`PVS_ARCHIV` / `DIRECTIMAGEIMPORT`)?
//! Bis dahin ist dies ein strukturiertes Gerüst, kein abgenommener Pfad.

use crate::error::{ConnectorError, Result};
use encoding_rs::WINDOWS_1252;
use std::path::{Path, PathBuf};

/// Patientenkontext, wie ihn media in der `[PATIENT]`-Sektion erwartet.
#[derive(Debug, Clone, Default)]
pub struct PatientContext {
    /// PVS-interne Patienten-ID (kommt PVS→uns über media).
    pub patient_id: String,
    pub last_name: String,
    pub first_name: String,
    /// Geburtsdatum `TT.MM.JJJJ`.
    pub birth_date: String,
}

/// Welche Art Dokument abgelegt wird (steuert ggf. Kategorie/Karteireiter).
#[derive(Debug, Clone, Copy)]
pub enum DocumentKind {
    Anamnese,
    Hkp,
}

impl DocumentKind {
    fn label(self) -> &'static str {
        match self {
            DocumentKind::Anamnese => "Anamnesebogen",
            DocumentKind::Hkp => "HKP",
        }
    }
}

pub struct ImportRequest<'a> {
    pub patient: &'a PatientContext,
    pub pdf_path: &'a Path,
    pub kind: DocumentKind,
}

/// Baut den `VDDS_MMO.INI`-Austauschtext (Windows-1252 wird beim Schreiben erzeugt).
pub fn build_mmo_ini(req: &ImportRequest) -> String {
    let p = req.patient;
    // Reihenfolge/Schlüssel an media 1.4 angelehnt — am echten PVS verifizieren.
    format!(
        "[PATIENT]\r\n\
PATID={patid}\r\n\
NAME={last}\r\n\
VORNAME={first}\r\n\
GEBDATUM={birth}\r\n\
[DOKUMENT]\r\n\
DATEI={file}\r\n\
TYP=PDF\r\n\
KATEGORIE={kind}\r\n\
BEMERKUNG=Erstellt über Praxishub\r\n",
        patid = p.patient_id,
        last = p.last_name,
        first = p.first_name,
        birth = p.birth_date,
        file = req.pdf_path.to_string_lossy(),
        kind = req.kind.label(),
    )
}

/// Schreibt die Austausch-INI ins (konfigurierte) Austausch-Verzeichnis und gibt
/// ihren Pfad zurück. `exchange_dir` = `ConnectorConfig::exchange_dir_path()`.
fn write_exchange_ini(req: &ImportRequest, exchange_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(exchange_dir)?;
    let path = exchange_dir.join("VDDS_MMO.INI");
    let text = build_mmo_ini(req);
    let (bytes, _, _) = WINDOWS_1252.encode(&text);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Legt ein PDF über das registrierte PVS-Import-Programm in die Akte.
///
/// `pvs_program` = Pfad zum media-Import-Executable des PVS (aus `VDDS_MMI.INI`).
/// `exchange_dir` = Austausch-Verzeichnis (Config; leer → Windows-Temp).
pub fn import_document(pvs_program: &Path, req: &ImportRequest, exchange_dir: &Path) -> Result<()> {
    if !req.pdf_path.exists() {
        return Err(ConnectorError::Vdds(format!(
            "PDF nicht gefunden: {}",
            req.pdf_path.display()
        )));
    }
    let ini_path = write_exchange_ini(req, exchange_dir)?;

    // Konvention: Aufruf `<pvs_program> <pfad-zur-MMO.ini>`. Exakte CLI-Signatur
    // (Schalter wie /import, Rückgabe-Konvention) am Z1 verifizieren.
    let status = std::process::Command::new(pvs_program)
        .arg(&ini_path)
        .status()
        .map_err(|e| ConnectorError::Vdds(format!("PVS-Programm nicht startbar: {e}")))?;

    if status.success() {
        Ok(())
    } else {
        Err(ConnectorError::Vdds(format!(
            "PVS-Import endete mit Status {status}"
        )))
    }
}

// ── Inbound: vom PVS als Media-Handler aufgerufen ────────────────────────────

/// Erkennt, ob ein CLI-Argument ein VDDS-media-Aufruf ist: Pfad auf eine
/// existierende `.ini`-Datei (der PVS ruft unser registriertes Modul so auf).
pub fn is_media_invocation(arg: &str) -> bool {
    arg.to_ascii_lowercase().ends_with(".ini") && Path::new(arg).is_file()
}

/// Liest den `[PATIENT]`-Kontext aus der vom PVS geschriebenen `VDDS_MMO.INI`
/// (Windows-1252).
pub fn parse_patient_from_request(ini_path: &Path) -> Result<PatientContext> {
    let bytes = std::fs::read(ini_path)?;
    let (text, _, _) = WINDOWS_1252.decode(&bytes);
    let ini = crate::vdds::ini::Ini::parse(&text);
    let get = |k: &str| ini.get("PATIENT", k).unwrap_or("").to_string();
    Ok(PatientContext {
        patient_id: get("PATID"),
        last_name: get("NAME"),
        first_name: get("VORNAME"),
        birth_date: get("GEBDATUM"),
    })
}

/// Einstiegspunkt, wenn der PVS unser Modul via VDDS-media aufruft
/// (`praxishub-connector.exe <pfad-zur-VDDS_MMO.INI>`). Parst den Patientenkontext.
///
/// ⚠️ **Z1-Pilot offen:** der weitere Ablauf — passende Praxishub-Dokumente für
/// diesen Patienten aus der Cloud holen und dem PVS zum Import zurückgeben — hängt
/// vom konkreten media-Antwortprotokoll des Z1 ab (Antwort-INI-Felder, Import-Trigger).
/// Hier wird der Kontext geparst und protokolliert; das Zurückschreiben folgt nach
/// Verifikation am echten PVS.
pub fn handle_invocation(ini_path: &Path) -> Result<PatientContext> {
    let patient = parse_patient_from_request(ini_path)?;
    tracing::info!(
        patid = %patient.patient_id,
        name = %patient.last_name,
        "VDDS-media: PVS-Aufruf für Patient erhalten"
    );
    Ok(patient)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parst_patient_aus_request_ini() {
        let path = std::env::temp_dir().join("praxishub_test_vdds_mmo_in.ini");
        std::fs::write(
            &path,
            b"[PATIENT]\r\nPATID=4711\r\nNAME=Mustermann\r\nVORNAME=Erika\r\nGEBDATUM=01.01.1980\r\n",
        )
        .unwrap();
        let pc = parse_patient_from_request(&path).unwrap();
        assert_eq!(pc.patient_id, "4711");
        assert_eq!(pc.last_name, "Mustermann");
        assert_eq!(pc.first_name, "Erika");
        assert_eq!(pc.birth_date, "01.01.1980");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn erkennt_keinen_media_aufruf_bei_flags() {
        assert!(!is_media_invocation("--autostart"));
        assert!(!is_media_invocation("/pfad/gibtsnicht.ini"));
    }

    #[test]
    fn mmo_ini_enthaelt_patient_und_dokument() {
        let patient = PatientContext {
            patient_id: "4711".into(),
            last_name: "Mustermann".into(),
            first_name: "Erika".into(),
            birth_date: "01.01.1980".into(),
        };
        let pdf = PathBuf::from("C:\\tmp\\anamnese.pdf");
        let req = ImportRequest { patient: &patient, pdf_path: &pdf, kind: DocumentKind::Anamnese };
        let ini = build_mmo_ini(&req);
        assert!(ini.contains("PATID=4711"));
        assert!(ini.contains("NAME=Mustermann"));
        assert!(ini.contains("KATEGORIE=Anamnesebogen"));
        assert!(ini.contains("anamnese.pdf"));
    }
}
