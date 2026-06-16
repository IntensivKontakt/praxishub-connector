//! Dokument-Ablage in die PVS-Akte über VDDS-media (BVS→PVS-Push, Stufe 6).
//!
//! Ablauf: der Connector schreibt eine Austausch-INI (`VDDS_MMO.INI`) mit
//! Patientenkontext + zu importierender PDF-Datei und ruft das vom PVS in
//! `VDDS_MMI.INI` registrierte Import-Programm auf (`MMOINFIMPORT`, bei Z1/CGM
//! `MmoInfIm.exe` — siehe [`crate::vdds::ini::pvs_import_program`]).
//!
//! **Patienten-Zuordnung — Kaskade** (vom Backend gewünscht, weil die Z1-`PATID`
//! in ~90 % der Fälle bereits bekannt ist):
//!   1. **PATID** — direkter, eindeutiger Push (unbeaufsichtigt).
//!   2. **Name + Geburtsdatum** — Fallback, wenn keine/abgelehnte PATID.
//!   3. **Variante A** — schlägt 1+2 fehl, bleibt das Dokument offen und wird
//!      abgelegt, sobald Z1 den Patienten öffnet und uns über `PATDATIMPORT` den
//!      Kontext (inkl. PATID) übergibt (siehe [`handle_invocation`]).
//!
//! ⚠️ **Am Z1-Pilot zu verifizieren:** Akzeptiert `MmoInfIm.exe` einen Push per
//! PATID bzw. per Name/Geburtsdatum unbeaufsichtigt, und nimmt es ein **PDF** in
//! die Dokumentenablage? CLI-Signatur/Rückgabe-Konvention ebenfalls am echten PVS
//! bestätigen. Bis dahin ist der Aufruf `<programm> <pfad-zur-MMO.ini>`.

use crate::error::{ConnectorError, Result};
use encoding_rs::WINDOWS_1252;
use std::path::{Path, PathBuf};

/// Patientenkontext, wie ihn media in der `[PATIENT]`-Sektion erwartet.
#[derive(Debug, Clone, Default)]
pub struct PatientContext {
    /// PVS-interne Patienten-ID (leer = unbekannt → Name/Geburtsdatum-Fallback).
    pub patient_id: String,
    pub last_name: String,
    pub first_name: String,
    /// Geburtsdatum `TT.MM.JJJJ`.
    pub birth_date: String,
}

impl PatientContext {
    /// Eindeutig per PVS-`PATID` identifizierbar?
    pub fn has_patid(&self) -> bool {
        !self.patient_id.trim().is_empty()
    }

    /// Genug für einen Name/Geburtsdatum-Fallback-Match?
    pub fn has_name_and_dob(&self) -> bool {
        !self.last_name.trim().is_empty() && !self.birth_date.trim().is_empty()
    }
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

    /// Aus der Backend-Kennung (`"anamnese"`/`"hkp"`); unbekannt → Anamnese.
    pub fn from_tag(tag: &str) -> Self {
        match tag.trim().to_ascii_lowercase().as_str() {
            "hkp" => DocumentKind::Hkp,
            _ => DocumentKind::Anamnese,
        }
    }
}

pub struct ImportRequest<'a> {
    pub patient: &'a PatientContext,
    pub pdf_path: &'a Path,
    pub kind: DocumentKind,
}

/// Ergebnis eines Ablage-Versuchs.
#[derive(Debug, PartialEq, Eq)]
pub enum FilingOutcome {
    /// PDF wurde an den PVS übergeben (Push erfolgreich). `matched_by` hält fest,
    /// WIE der Patient getroffen wurde ("patient_id" | "name_dob") — die Cloud
    /// quittiert damit „für genau diesen Patienten".
    Filed { matched_by: &'static str },
    /// Unbeaufsichtigte Ablage (PATID + Name/Geburtsdatum) nicht möglich — das
    /// Dokument bleibt offen für Variante A (Ablage beim nächsten Z1-Aufruf).
    Deferred(String),
}

/// Baut den `VDDS_MMO.INI`-Austauschtext. `PATID` wird nur geschrieben, wenn
/// bekannt — so erzwingt der Fallback eine Name/Geburtsdatum-Identifikation.
pub fn build_mmo_ini(req: &ImportRequest) -> String {
    let p = req.patient;
    let mut s = String::from("[PATIENT]\r\n");
    if p.has_patid() {
        s.push_str(&format!("PATID={}\r\n", p.patient_id.trim()));
    }
    s.push_str(&format!("NAME={}\r\n", p.last_name));
    s.push_str(&format!("VORNAME={}\r\n", p.first_name));
    s.push_str(&format!("GEBDATUM={}\r\n", p.birth_date));
    s.push_str("[DOKUMENT]\r\n");
    s.push_str(&format!("DATEI={}\r\n", req.pdf_path.to_string_lossy()));
    s.push_str("TYP=PDF\r\n");
    s.push_str(&format!("KATEGORIE={}\r\n", req.kind.label()));
    s.push_str("BEMERKUNG=Erstellt über Praxishub\r\n");
    s
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

/// Ein einzelner Import-Aufruf: INI schreiben, PVS-Programm starten.
/// `Ok(true)` = PVS meldete Erfolg, `Ok(false)` = Programm lief, lehnte aber ab
/// (z. B. Patient nicht gefunden), `Err` = Programm gar nicht startbar (Konfig).
fn run_import(import_program: &Path, req: &ImportRequest, exchange_dir: &Path) -> Result<bool> {
    let ini_path = write_exchange_ini(req, exchange_dir)?;
    // Konvention: `<programm> <pfad-zur-MMO.ini>` — exakte CLI-Signatur am Z1 verifizieren.
    let status = std::process::Command::new(import_program)
        .arg(&ini_path)
        .status()
        .map_err(|e| ConnectorError::Vdds(format!("PVS-Programm nicht startbar: {e}")))?;
    Ok(status.success())
}

/// Legt ein PDF über das PVS-Importmodul in die Akte — mit der Kaskade
/// **PATID → Name/Geburtsdatum → (offen für Variante A)**.
///
/// `import_program` = aus `VDDS_MMI.INI` ausgelesenes `MMOINFIMPORT` (Z1: `MmoInfIm.exe`).
pub fn file_document(
    import_program: &Path,
    req: &ImportRequest,
    exchange_dir: &Path,
) -> Result<FilingOutcome> {
    if !req.pdf_path.exists() {
        return Err(ConnectorError::Vdds(format!(
            "PDF nicht gefunden: {}",
            req.pdf_path.display()
        )));
    }

    // 1) PATID-Versuch (eindeutig, unbeaufsichtigt).
    if req.patient.has_patid() && run_import(import_program, req, exchange_dir)? {
        tracing::info!(patid = %req.patient.patient_id, "VDDS-media: Dokument per PATID abgelegt");
        return Ok(FilingOutcome::Filed { matched_by: "patient_id" });
    }

    // 2) Name/Geburtsdatum-Fallback (PATID bewusst weggelassen → erzwingt Match).
    if req.patient.has_name_and_dob() {
        let by_name = PatientContext {
            patient_id: String::new(),
            ..req.patient.clone()
        };
        let req2 = ImportRequest {
            patient: &by_name,
            pdf_path: req.pdf_path,
            kind: req.kind,
        };
        if run_import(import_program, &req2, exchange_dir)? {
            tracing::info!(
                name = %req.patient.last_name,
                "VDDS-media: Dokument per Name/Geburtsdatum abgelegt"
            );
            return Ok(FilingOutcome::Filed { matched_by: "name_dob" });
        }
    }

    // 3) Variante A: offen lassen, bis Z1 den Patienten öffnet.
    Ok(FilingOutcome::Deferred(
        "unbeaufsichtigte Ablage nicht möglich – wartet auf Z1-Patientenkontext".into(),
    ))
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
/// (`praxishub-connector.exe <pfad-zur-VDDS_MMO.INI>`). Parst den Patientenkontext
/// (inkl. der vom PVS vergebenen PATID) — das ist die Grundlage für Variante A:
/// offene Dokumente dieses Patienten lassen sich nun mit echter PATID ablegen.
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

    fn patient_full() -> PatientContext {
        PatientContext {
            patient_id: "4711".into(),
            last_name: "Mustermann".into(),
            first_name: "Erika".into(),
            birth_date: "01.01.1980".into(),
        }
    }

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
    fn mmo_ini_enthaelt_patid_wenn_bekannt() {
        let patient = patient_full();
        let pdf = PathBuf::from("C:\\tmp\\anamnese.pdf");
        let req = ImportRequest { patient: &patient, pdf_path: &pdf, kind: DocumentKind::Anamnese };
        let ini = build_mmo_ini(&req);
        assert!(ini.contains("PATID=4711"));
        assert!(ini.contains("NAME=Mustermann"));
        assert!(ini.contains("KATEGORIE=Anamnesebogen"));
        assert!(ini.contains("anamnese.pdf"));
    }

    #[test]
    fn mmo_ini_ohne_patid_wenn_unbekannt() {
        let patient = PatientContext { patient_id: String::new(), ..patient_full() };
        let pdf = PathBuf::from("C:\\tmp\\hkp.pdf");
        let req = ImportRequest { patient: &patient, pdf_path: &pdf, kind: DocumentKind::Hkp };
        let ini = build_mmo_ini(&req);
        assert!(!ini.contains("PATID="));
        assert!(ini.contains("NAME=Mustermann"));
        assert!(ini.contains("KATEGORIE=HKP"));
    }

    // Die Kaskade über echte Prozessaufrufe testen wir mit /bin/true|false (Unix).
    #[cfg(unix)]
    fn dummy_pdf() -> PathBuf {
        let p = std::env::temp_dir().join("praxishub_test_doc.pdf");
        std::fs::write(&p, b"%PDF-1.4 test").unwrap();
        p
    }

    #[test]
    #[cfg(unix)]
    fn kaskade_filed_wenn_programm_erfolg_meldet() {
        let pdf = dummy_pdf();
        let patient = patient_full();
        let req = ImportRequest { patient: &patient, pdf_path: &pdf, kind: DocumentKind::Anamnese };
        let out = file_document(Path::new("/bin/true"), &req, &std::env::temp_dir()).unwrap();
        assert_eq!(out, FilingOutcome::Filed { matched_by: "patient_id" });
    }

    #[test]
    #[cfg(unix)]
    fn kaskade_deferred_wenn_programm_immer_ablehnt() {
        let pdf = dummy_pdf();
        let patient = patient_full(); // PATID + Name/DOB → beide Versuche scheitern an /bin/false
        let req = ImportRequest { patient: &patient, pdf_path: &pdf, kind: DocumentKind::Anamnese };
        let out = file_document(Path::new("/bin/false"), &req, &std::env::temp_dir()).unwrap();
        assert!(matches!(out, FilingOutcome::Deferred(_)));
    }

    #[test]
    fn kaskade_fehlt_pdf_ist_fehler() {
        let patient = patient_full();
        let missing = PathBuf::from("/pfad/gibtsnicht-12345.pdf");
        let req = ImportRequest { patient: &patient, pdf_path: &missing, kind: DocumentKind::Anamnese };
        assert!(file_document(Path::new("/bin/true"), &req, &std::env::temp_dir()).is_err());
    }
}
