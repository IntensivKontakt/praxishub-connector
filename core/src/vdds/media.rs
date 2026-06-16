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
    /// Geburtsdatum. VDDS-media liefert es als `CCYYMMDD` (z. B. `19800101`,
    /// Tabelle 3 `BIRTHDAY=`). Für MMOINFIMPORT irrelevant (Match über PATID);
    /// nur fürs Dokument-Matching gegen die Cloud — dort muss das Format passen.
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
    /// VDDS-media-Objekttyp: `(Klartext für TYPE=, Nummer für TYPENR= gemäß Tabelle 15)`.
    fn media_type(self) -> (&'static str, u32) {
        match self {
            // Heil- und Kostenplan (Tabelle 15, Nr. 14).
            DocumentKind::Hkp => ("Heil- und Kostenplan", 14),
            // Anamnesebogen → Formular (Tabelle 15, Nr. 10).
            DocumentKind::Anamnese => ("Formular", 10),
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

/// VDDS-media-Zielangaben aus der `VDDS_MMI.INI` (Sektionsnamen für den Push).
#[derive(Debug, Clone)]
pub struct VddsTarget {
    /// `PVS=` — Sektion der empfangenden PVS/Archiv (die mit `MMOINFIMPORT`).
    pub pvs_section: String,
    /// `FROMPVS=` — patientenführende PVS-Sektion.
    pub from_pvs_section: String,
    /// `BVS=` — unsere eigene BVS-Sektion (`PRAXISHUB`).
    pub bvs_section: String,
    /// `PRXNR=` — Praxisnummer (Default „1", falls unbekannt).
    pub prxnr: String,
}

impl VddsTarget {
    /// Liest die nötigen Sektionsnamen aus der `VDDS_MMI.INI`. Fehlt etwas, werden
    /// Defaults benutzt — am Z1-Pilot via `--push-test`-Diagnose zu bestätigen.
    pub fn from_mmi_ini() -> Self {
        use crate::vdds::ini;
        let bytes = std::fs::read(ini::default_ini_path()).unwrap_or_default();
        let (text, _, _) = WINDOWS_1252.decode(&bytes);
        let mmi = ini::Ini::parse(&text);
        // Der registrierte PVS-Sektionsname (`[PVS] NAME1`, z. B. `CDP_Z1`) — NICHT
        // die Archiv-Sektion. Fällt NAME1 weg, ersatzweise die Archiv-Sektion.
        let pvs_name = mmi
            .get("PVS", "NAME1")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| mmi.get("PVS", "ARCHIV"))
            .unwrap_or("")
            .trim()
            .to_string();
        VddsTarget {
            // PVS=/FROMPVS= = dieselbe (patientenführende) PVS bei Ein-PVS-Praxen.
            pvs_section: pvs_name.clone(),
            from_pvs_section: pvs_name,
            bvs_section: ini::SECTION.to_string(),
            prxnr: "1".to_string(),
        }
    }
}

/// Baut den `VDDS_MMO.INI`-Austauschtext für den **MMOINFIMPORT-Push**
/// (VDDS-media 1.4, Tabelle 5–7). Patient via `[PATID]` (PATID Pflicht,
/// englische Feldnamen), Objekt als direkt übergebene Datei per `IMAGEDATA=`
/// (`EXT=PDF`, Dokument → `COLORTYPE=LINEART`). `READY/ERRORLEVEL` als Handshake.
pub fn build_mmo_ini(req: &ImportRequest, target: &VddsTarget) -> String {
    let p = req.patient;
    let (type_text, typenr) = req.kind.media_type();
    let now = chrono::Local::now();
    let date = now.format("%Y%m%d").to_string(); // CCYYMMDD
    let mmoid = now.format("PH%Y%m%d%H%M%S").to_string(); // eindeutige Objekt-ID
    let mut s = String::new();
    // Kopf-/Patientensektion (Tabelle 5).
    s.push_str("[PATID]\r\n");
    s.push_str(&format!("PVS={}\r\n", target.pvs_section));
    s.push_str(&format!("BVS={}\r\n", target.bvs_section));
    s.push_str(&format!("FROMPVS={}\r\n", target.from_pvs_section));
    s.push_str(&format!("PRXNR={}\r\n", target.prxnr));
    // Tabelle 5 kennt im MMOINFIMPORT-File KEINE Namensfelder — Zuordnung rein
    // über die PATID (der Name/Geburtsdatum-Weg ist hier prinzipiell nicht möglich).
    s.push_str(&format!("PATID={}\r\n", p.patient_id.trim()));
    s.push_str("ERRORLEVEL=0\r\n");
    s.push_str("READY=0\r\n");
    // Objektliste (Tabelle 6).
    s.push_str("[MMOS]\r\n");
    s.push_str("COUNT=1\r\n");
    // Objekt 1 (Tabelle 7) — das PDF.
    s.push_str("[MMO1]\r\n");
    s.push_str(&format!("MMOID={mmoid}\r\n"));
    s.push_str(&format!("PRXNR={}\r\n", target.prxnr));
    s.push_str(&format!("TYPE={type_text}\r\n"));
    s.push_str(&format!("TYPENR={typenr}\r\n"));
    s.push_str("EXT=PDF\r\n");
    s.push_str("COLORTYPE=LINEART\r\n");
    s.push_str(&format!("DATE={date}\r\n"));
    s.push_str("COMMENT=Erstellt über Praxishub\r\n");
    s.push_str(&format!("IMAGEDATA={}\r\n", req.pdf_path.to_string_lossy()));
    s
}

/// Schreibt die Austausch-INI ins (konfigurierte) Austausch-Verzeichnis und gibt
/// ihren Pfad zurück. `exchange_dir` = `ConnectorConfig::exchange_dir_path()`.
fn write_exchange_ini(req: &ImportRequest, exchange_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(exchange_dir)?;
    let path = exchange_dir.join("VDDS_MMO.INI");
    let target = VddsTarget::from_mmi_ini();
    let text = build_mmo_ini(req, &target);
    let (bytes, _, _) = WINDOWS_1252.encode(&text);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Liest einen Feldwert (egal in welcher Sektion) aus einer INI-Datei.
fn read_ini_field(ini_path: &Path, key: &str) -> Option<String> {
    let bytes = std::fs::read(ini_path).ok()?;
    let (text, _, _) = WINDOWS_1252.decode(&bytes);
    text.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(key)
            .then(|| v.trim().to_string())
    })
}

/// Erfolg eines Imports anhand des VDDS-Handshakes bewerten: Das PVS-Modul setzt
/// als **letzte** Aktion `READY=1` und trägt `ERRORLEVEL` ein (0 = ok). Nur dann
/// ist die Aussage verlässlich. Hat es den Handshake nicht gesetzt (z. B. rein
/// synchrones Modul), fällt die Bewertung auf den Prozess-Exit-Code zurück.
fn import_succeeded(ini_path: &Path, exit_success: bool) -> bool {
    match read_ini_field(ini_path, "READY").as_deref() {
        Some("1") => read_ini_field(ini_path, "ERRORLEVEL").as_deref() == Some("0"),
        _ => exit_success,
    }
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
    // Erfolg über den READY/ERRORLEVEL-Handshake bewerten (Fallback: Exit-Code).
    Ok(import_succeeded(&ini_path, status.success()))
}

/// Diagnose eines **einzelnen** Import-Aufrufs — für `--push-test` am echten Z1.
/// Liefert Exit-Code **und** das, was `MmoInfIm` in die Austausch-INI
/// zurückschreibt (v. a. ein `ERRORLEVEL` — die VDDS-media-Erfolgs-/Fehler-
/// Konvention) sowie die Dateien im Austauschordner. Verändert die normale
/// Erfolgslogik NICHT, sondern legt offen, *warum* Z1 (nicht) übernimmt.
#[derive(Debug)]
pub struct ImportDiagnostics {
    pub exit_code: Option<i32>,
    pub exit_success: bool,
    /// `READY`-Wert nach dem Aufruf (`1` = PVS-Handshake abgeschlossen).
    pub ready: Option<String>,
    /// `ERRORLEVEL`-Wert aus der zurückgelesenen INI (egal in welcher Sektion).
    pub errorlevel: Option<String>,
    /// `ERRORTEXT`/`ERROR-TEXT` — Klartext-Fehlergrund, den das PVS zurückschreibt.
    pub errortext: Option<String>,
    /// Die INI, die wir an `MmoInfIm` übergeben haben.
    pub sent_ini: String,
    /// Inhalt der Austausch-INI NACH dem Aufruf (zeigt die Antwort von `MmoInfIm`).
    pub ini_after: String,
    /// Dateinamen (+ Größe) im Austauschordner nach dem Aufruf.
    pub exchange_files: Vec<String>,
}

/// Führt genau einen Import-Aufruf aus und liest danach zurück, was hinterlassen
/// wurde. Rein diagnostisch (keine Kaskade, keine Erfolgsbewertung).
pub fn import_once_diagnostic(
    import_program: &Path,
    req: &ImportRequest,
    exchange_dir: &Path,
) -> Result<ImportDiagnostics> {
    if !req.pdf_path.exists() {
        return Err(ConnectorError::Vdds(format!(
            "PDF nicht gefunden: {}",
            req.pdf_path.display()
        )));
    }
    let sent_ini = build_mmo_ini(req, &VddsTarget::from_mmi_ini());
    let ini_path = write_exchange_ini(req, exchange_dir)?;
    let status = std::process::Command::new(import_program)
        .arg(&ini_path)
        .status()
        .map_err(|e| ConnectorError::Vdds(format!("PVS-Programm nicht startbar: {e}")))?;

    let ini_after = match std::fs::read(&ini_path) {
        Ok(bytes) => WINDOWS_1252.decode(&bytes).0.into_owned(),
        Err(e) => format!("(INI nach Aufruf nicht lesbar: {e})"),
    };
    // READY/ERRORLEVEL aus der zurückgeschriebenen INI herausfischen.
    let field = |key: &str| {
        ini_after.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            k.trim().eq_ignore_ascii_case(key).then(|| v.trim().to_string())
        })
    };
    let ready = field("READY");
    let errorlevel = field("ERRORLEVEL");
    let errortext = field("ERRORTEXT").or_else(|| field("ERROR-TEXT"));
    let exchange_files = std::fs::read_dir(exchange_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| {
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    format!("{} ({} B)", e.file_name().to_string_lossy(), size)
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ImportDiagnostics {
        exit_code: status.code(),
        exit_success: status.success(),
        ready,
        errorlevel,
        errortext,
        sent_ini,
        ini_after,
        exchange_files,
    })
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

    // MMOINFIMPORT ordnet AUSSCHLIESSLICH über die PATID zu — Tabelle 5 kennt keinen
    // Namens-/Geburtsdatum-Match. Greift die PATID nicht, bleibt nur Variante A.
    if req.patient.has_patid() && run_import(import_program, req, exchange_dir)? {
        tracing::info!(patid = %req.patient.patient_id, "VDDS-media: Dokument per PATID abgelegt");
        return Ok(FilingOutcome::Filed { matched_by: "patient_id" });
    }

    // Variante A: offen lassen, bis Z1 den Patienten öffnet und uns die PATID übergibt.
    Ok(FilingOutcome::Deferred(
        "PATID greift (noch) nicht – wartet auf Z1-Patientenkontext (Variante A)".into(),
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
    // Z1 kann die Patientensektion `[PATID]` oder `[PATIENT]` nennen und englische
    // (LASTNAME/FIRSTNAME) wie deutsche (NAME/VORNAME) Feldnamen verwenden.
    let first = |keys: &[&str]| -> String {
        keys.iter()
            .find_map(|k| {
                let v = ini.get("PATID", k).or_else(|| ini.get("PATIENT", k))?;
                (!v.trim().is_empty()).then(|| v.to_string())
            })
            .unwrap_or_default()
    };
    Ok(PatientContext {
        patient_id: first(&["PATID"]),
        last_name: first(&["LASTNAME", "NAME"]),
        first_name: first(&["FIRSTNAME", "VORNAME"]),
        birth_date: first(&["BIRTHDATE", "BIRTHDAY", "GEBDATUM"]),
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

    fn test_target() -> VddsTarget {
        VddsTarget {
            pvs_section: "CDP_Z1".into(),
            from_pvs_section: "CDP_Z1".into(),
            bvs_section: "PRAXISHUB".into(),
            prxnr: "1".into(),
        }
    }

    #[test]
    fn mmo_ini_anamnese_schema_ist_vdds_konform() {
        let patient = patient_full();
        let pdf = PathBuf::from("C:\\tmp\\anamnese.pdf");
        let req = ImportRequest { patient: &patient, pdf_path: &pdf, kind: DocumentKind::Anamnese };
        let ini = build_mmo_ini(&req, &test_target());
        // Kopfsektion: Zuordnung NUR über PATID, mit PVS/BVS/FROMPVS/PRXNR.
        assert!(ini.contains("[PATID]\r\nPVS=CDP_Z1\r\n"));
        assert!(ini.contains("PATID=4711"));
        assert!(ini.contains("BVS=PRAXISHUB"));
        assert!(ini.contains("FROMPVS=CDP_Z1"));
        assert!(ini.contains("PRXNR=1"));
        // Objektsektion mit direkter Dateiübergabe.
        assert!(ini.contains("[MMOS]"));
        assert!(ini.contains("COUNT=1"));
        assert!(ini.contains("[MMO1]"));
        assert!(ini.contains("TYPENR=10")); // Formular
        assert!(ini.contains("EXT=PDF"));
        assert!(ini.contains("COLORTYPE=LINEART"));
        assert!(ini.contains("IMAGEDATA=C:\\tmp\\anamnese.pdf"));
        // Tabelle 5 hat KEINE Namensfelder, und alte (falsche) Schlüssel sind weg.
        assert!(!ini.contains("LASTNAME"));
        assert!(!ini.contains("FIRSTNAME"));
        assert!(!ini.contains("KATEGORIE"));
        assert!(!ini.contains("DATEI="));
        assert!(!ini.contains("GEBDATUM"));
    }

    #[test]
    fn mmo_ini_hkp_nutzt_typenr_14() {
        let patient = patient_full();
        let pdf = PathBuf::from("C:\\tmp\\hkp.pdf");
        let req = ImportRequest { patient: &patient, pdf_path: &pdf, kind: DocumentKind::Hkp };
        let ini = build_mmo_ini(&req, &test_target());
        assert!(ini.contains("TYPENR=14")); // Heil- und Kostenplan
        assert!(ini.contains("TYPE=Heil- und Kostenplan"));
        assert!(ini.contains("EXT=PDF"));
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
