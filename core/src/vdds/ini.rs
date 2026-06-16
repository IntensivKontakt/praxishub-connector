//! `VDDS_MMI.INI` — lokale Selbst-Registrierung als BVS-Modul + Auslesen des
//! PVS-Importmoduls (für den BVS→PVS-Dokumenten-Push, VDDS-media Stufe 6).
//!
//! Laut VDDS-Konzept trägt sich jede teilnehmende Software lokal in die
//! `VDDS_MMI.INI` ein (freier Wettbewerb, keine Whitelist). Der PVS ruft dann
//! generisch die registrierten Module auf.
//!
//! **Schema (an echter Z1/CGM-`VDDS_MMI.INI` verifiziert):** Die Datei führt die
//! teilnehmenden Systeme NICHT über einen `[SYSTEMS]`-Index, sondern über die
//! Index-Abschnitte `[PVS]` und `[BVS]` mit `NAME1=`, `NAME2=`-Schlüsseln, die je
//! auf einen gleichnamigen Detail-Abschnitt zeigen. Beispiel (gekürzt):
//! ```ini
//! [PVS]
//! NAME1=CDP_Z1
//! ARCHIV=PVS_ARCHIV
//! [PVS_ARCHIV]
//! MMOINFIMPORT=C:\CGM\PRAXIS~1\Client\VDDS\MmoInfIm.exe   ; PVS-Importmodul
//! [BVS]
//! NAME1=CONVIS_PRAXISARCHIV
//! NAME2=PAVDTQ_Sidexis
//! ```
//! Praxishub registriert sich als zusätzliches **BVS** (`[BVS] NAMEk=PRAXISHUB`
//! + Detail-Abschnitt `[PRAXISHUB]`), damit Z1 uns bei geöffnetem Patienten den
//! Kontext über `PATDATIMPORT` übergeben kann (Variante A). Den Dokumenten-Push
//! in die Akte fahren wir, indem wir das vom PVS registrierte `MMOINFIMPORT`
//! aufrufen — siehe [`pvs_import_program`].
//!
//! Die Datei ist **Windows-1252**-kodiert und liegt traditionell im Windows-
//! Verzeichnis (`%WINDIR%\VDDS_MMI.INI`); Pfad per Env `VDDS_INI` überschreibbar.

use crate::error::Result;
use encoding_rs::WINDOWS_1252;
use std::path::{Path, PathBuf};

/// Abschnittsname unseres Moduls (freie Namen-ID).
pub const SECTION: &str = "PRAXISHUB";
/// Index-Abschnitt der Bildverwaltungssysteme — hier tragen wir uns ein.
const BVS_INDEX: &str = "BVS";
/// Index-Abschnitt der Praxisverwaltungssysteme (Z1) — hier steht `ARCHIV=…`.
const PVS_INDEX: &str = "PVS";
/// Altbestand aus v0.1.x (fälschlich `[SYSTEMS]`); beim Deregistrieren miträumen.
const LEGACY_SYSTEMS: &str = "SYSTEMS";
/// VDDS-media-Schnittstellenversion, die wir bedienen (NICHT die App-Version).
const MEDIA_VERSION: &str = "1.3";

pub struct VddsRegistration {
    /// Vollpfad zur ausführbaren Media-Handler-.exe (vom PVS aufgerufen).
    pub program_path: PathBuf,
    /// Installationsverzeichnis.
    pub install_dir: PathBuf,
}

/// Standardpfad der `VDDS_MMI.INI`.
pub fn default_ini_path() -> PathBuf {
    if let Ok(p) = std::env::var("VDDS_INI") {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        PathBuf::from(windir).join("VDDS_MMI.INI")
    }
    #[cfg(not(windows))]
    {
        // Auf Nicht-Windows (Dev/Mac) nie ins System schreiben.
        std::env::temp_dir().join("VDDS_MMI.INI")
    }
}

// ── Datei-Ebene (mit Encoding) ───────────────────────────────────────────────

fn read_ini(path: &Path) -> Result<Ini> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let (text, _, _) = WINDOWS_1252.decode(&bytes);
            Ok(Ini::parse(&text))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ini::default()),
        Err(e) => Err(e.into()),
    }
}

fn write_ini(path: &Path, ini: &Ini) -> Result<()> {
    let text = ini.to_text();
    let (bytes, _, _) = WINDOWS_1252.encode(&text);
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Trägt Praxishub in die INI ein (idempotent).
pub fn register(ini_path: &Path, reg: &VddsRegistration) -> Result<()> {
    let mut ini = read_ini(ini_path)?;
    register_in(&mut ini, reg);
    write_ini(ini_path, &ini)
}

/// Entfernt den Praxishub-Eintrag wieder (bei Deinstallation).
pub fn unregister(ini_path: &Path) -> Result<()> {
    let mut ini = read_ini(ini_path)?;
    unregister_in(&mut ini);
    write_ini(ini_path, &ini)
}

pub fn is_registered(ini_path: &Path) -> Result<bool> {
    Ok(read_ini(ini_path)?.has_section(SECTION))
}

/// Liest das vom PVS registrierte **Importmodul** (`MMOINFIMPORT`) aus einer
/// echten `VDDS_MMI.INI`: `[PVS].ARCHIV` zeigt auf den Archiv-Abschnitt, dort
/// steht `MMOINFIMPORT=…`. Genau dieses Programm rufen wir für den
/// Dokumenten-Push in die Z1-Akte auf. `Ok(None)` = Z1 bietet keinen Info-Import.
pub fn read_pvs_import_program(ini_path: &Path) -> Result<Option<PathBuf>> {
    Ok(pvs_import_program(&read_ini(ini_path)?))
}

// ── reine In-Memory-Logik (unit-testbar) ─────────────────────────────────────

pub fn register_in(ini: &mut Ini, reg: &VddsRegistration) {
    // Im [BVS]-Index registrieren (nächster freier NAMEk), falls noch nicht da.
    if ini.indexed_key_for(BVS_INDEX, SECTION).is_none() {
        let key = ini.next_indexed_key(BVS_INDEX);
        ini.set(BVS_INDEX, &key, SECTION);
    }
    let prog = reg.program_path.to_string_lossy();
    let dir = reg.install_dir.to_string_lossy();
    // Detail-Abschnitt als BVS-Modul. PATDATIMPORT = unsere .exe, die Z1 bei
    // geöffnetem Patienten mit dem Patientenkontext aufruft (Variante A).
    ini.set(SECTION, "NAME", "Praxishub Connector");
    ini.set(SECTION, "PATDATIMPORT", &prog);
    ini.set(SECTION, "PATDATIMPORT_OS", "1");
    ini.set(SECTION, "PATDATIMPORT_EVENT", "");
    // MMOEXPORT = unser Bildkopie-/Dokument-Export (Stufe 4). Nötig, weil ConVis
    // keinen DIRECTIMAGEIMPORT bietet: Nach unserem MMOINFIMPORT-Push HOLT der PVS
    // die Dokumentkopie per Pull über genau dieses Modul ab (VDDS-media Tabelle 8/9,
    // beantwortet von [`crate::vdds::media::handle_export_request`]).
    ini.set(SECTION, "MMOEXPORT", &prog);
    ini.set(SECTION, "MMOEXPORT_OS", "1");
    ini.set(SECTION, "MMOEXPORT_EVENT", "");
    ini.set(SECTION, "PFAD", &dir);
    // Realisierte BVS-Stufen: Patientenübergabe (1) + Bildkopie-Export/Pull (4).
    ini.set(SECTION, "STAGES", "14");
    ini.set(SECTION, "VERSION", MEDIA_VERSION);
}

pub fn unregister_in(ini: &mut Ini) {
    ini.remove_indexed_value(BVS_INDEX, SECTION);
    ini.remove_indexed_value(PVS_INDEX, SECTION); // falls je versehentlich als PVS
    ini.remove_indexed_value(LEGACY_SYSTEMS, SECTION); // Altbestand v0.1.x
    ini.remove_section(SECTION);
}

/// Ermittelt das PVS-Importmodul (`MMOINFIMPORT`) aus einer geparsten INI.
pub fn pvs_import_program(ini: &Ini) -> Option<PathBuf> {
    let archiv = ini.get(PVS_INDEX, "ARCHIV")?;
    let prog = ini.get(archiv, "MMOINFIMPORT")?;
    let prog = prog.trim();
    if prog.is_empty() {
        None
    } else {
        Some(PathBuf::from(prog))
    }
}

// ── Minimaler INI-Parser (Reihenfolge-erhaltend) ─────────────────────────────

#[derive(Default)]
pub struct Ini {
    sections: Vec<Section>,
}

struct Section {
    name: String,
    entries: Vec<(String, String)>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut sections: Vec<Section> = Vec::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let name = line[1..line.len() - 1].trim().to_string();
                sections.push(Section { name, entries: Vec::new() });
            } else if let Some((k, v)) = line.split_once('=') {
                if let Some(sec) = sections.last_mut() {
                    sec.entries.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
        }
        Self { sections }
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for sec in &self.sections {
            out.push('[');
            out.push_str(&sec.name);
            out.push_str("]\r\n");
            for (k, v) in &sec.entries {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push_str("\r\n");
            }
            out.push_str("\r\n");
        }
        out
    }

    pub fn has_section(&self, name: &str) -> bool {
        self.sections.iter().any(|s| s.name.eq_ignore_ascii_case(name))
    }

    fn section_mut(&mut self, name: &str) -> &mut Section {
        if let Some(idx) = self.sections.iter().position(|s| s.name.eq_ignore_ascii_case(name)) {
            &mut self.sections[idx]
        } else {
            self.sections.push(Section { name: name.to_string(), entries: Vec::new() });
            self.sections.last_mut().unwrap()
        }
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let sec = self.section_mut(section);
        if let Some(e) = sec.entries.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            e.1 = value.to_string();
        } else {
            sec.entries.push((key.to_string(), value.to_string()));
        }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(section))?
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn remove_section(&mut self, name: &str) -> bool {
        let before = self.sections.len();
        self.sections.retain(|s| !s.name.eq_ignore_ascii_case(name));
        self.sections.len() != before
    }

    /// Schlüssel im Index-Abschnitt (`NAME1`, `NAME2`, …), dessen Wert == `value`.
    fn indexed_key_for(&self, index: &str, value: &str) -> Option<String> {
        let sec = self.sections.iter().find(|s| s.name.eq_ignore_ascii_case(index))?;
        sec.entries
            .iter()
            .find(|(_, v)| v.eq_ignore_ascii_case(value))
            .map(|(k, _)| k.clone())
    }

    /// Nächster freier `NAMEk`-Schlüssel im Index-Abschnitt (z. B. `NAME3`).
    fn next_indexed_key(&self, index: &str) -> String {
        let max = self
            .sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(index))
            .map(|sec| {
                sec.entries
                    .iter()
                    .filter_map(|(k, _)| {
                        let up = k.to_ascii_uppercase();
                        up.strip_prefix("NAME").and_then(|n| n.parse::<u32>().ok())
                    })
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        format!("NAME{}", max + 1)
    }

    /// Entfernt aus einem Index-Abschnitt den Eintrag mit Wert == `value`.
    fn remove_indexed_value(&mut self, index: &str, value: &str) {
        if let Some(sec) = self.sections.iter_mut().find(|s| s.name.eq_ignore_ascii_case(index)) {
            sec.entries.retain(|(_, v)| !v.eq_ignore_ascii_case(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gekürzter, aber struktur-echter Auszug einer Z1/CGM-`VDDS_MMI.INI`.
    const Z1_INI: &str = "[PVS]\r\n\
NAME1=CDP_Z1\r\n\
ARCHIV=PVS_ARCHIV\r\n\
[CDP_Z1]\r\n\
NAME=CompuDENT Z1\r\n\
[PVS_ARCHIV]\r\n\
NAME=PraxisArchiv - ConVis\r\n\
MMOINFIMPORT=C:\\CGM\\PRAXIS~1\\Client\\VDDS\\MmoInfIm.exe\r\n\
STAGES=123456\r\n\
[BVS]\r\n\
NAME1=CONVIS_PRAXISARCHIV\r\n\
NAME2=PAVDTQ_Sidexis\r\n\
[CONVIS_PRAXISARCHIV]\r\n\
NAME=Erfassung über PraxisArchiv - ConVis\r\n";

    fn reg() -> VddsRegistration {
        VddsRegistration {
            program_path: PathBuf::from("C:\\Apps\\praxishub-connector.exe"),
            install_dir: PathBuf::from("C:\\Apps"),
        }
    }

    #[test]
    fn liest_mmoinfimport_aus_echter_z1_ini() {
        let ini = Ini::parse(Z1_INI);
        assert_eq!(
            pvs_import_program(&ini),
            Some(PathBuf::from("C:\\CGM\\PRAXIS~1\\Client\\VDDS\\MmoInfIm.exe"))
        );
    }

    #[test]
    fn register_traegt_bvs_und_detailsektion_ein() {
        let mut ini = Ini::parse(Z1_INI);
        register_in(&mut ini, &reg());
        // nächster freier NAMEk im [BVS]-Index ist NAME3
        assert_eq!(ini.indexed_key_for(BVS_INDEX, SECTION).as_deref(), Some("NAME3"));
        assert_eq!(ini.get(BVS_INDEX, "NAME3"), Some(SECTION));
        assert!(ini.has_section(SECTION));
        assert_eq!(
            ini.get(SECTION, "PATDATIMPORT"),
            Some("C:\\Apps\\praxishub-connector.exe")
        );
        // MMOEXPORT (Pull-Modul) muss registriert sein, sonst kann ConVis unsere
        // Dokumentkopie nach dem Push nicht abholen.
        assert_eq!(
            ini.get(SECTION, "MMOEXPORT"),
            Some("C:\\Apps\\praxishub-connector.exe")
        );
        assert_eq!(ini.get(SECTION, "STAGES"), Some("14"));
        assert_eq!(ini.get(SECTION, "VERSION"), Some(MEDIA_VERSION));
        // bestehende Z1-Einträge bleiben unangetastet
        assert_eq!(ini.get(BVS_INDEX, "NAME1"), Some("CONVIS_PRAXISARCHIV"));
        assert_eq!(pvs_import_program(&ini).is_some(), true);
    }

    #[test]
    fn register_ist_idempotent() {
        let mut ini = Ini::parse(Z1_INI);
        register_in(&mut ini, &reg());
        register_in(&mut ini, &reg());
        // genau ein Index-Eintrag, der auf PRAXISHUB zeigt
        let count = ini
            .sections
            .iter()
            .find(|s| s.name == BVS_INDEX)
            .map(|s| s.entries.iter().filter(|(_, v)| v == SECTION).count())
            .unwrap_or(0);
        assert_eq!(count, 1);
    }

    #[test]
    fn register_ohne_vorhandenen_bvs_index_legt_name1_an() {
        let mut ini = Ini::default();
        register_in(&mut ini, &reg());
        assert_eq!(ini.get(BVS_INDEX, "NAME1"), Some(SECTION));
        assert!(ini.has_section(SECTION));
    }

    #[test]
    fn unregister_entfernt_index_und_section() {
        let mut ini = Ini::parse(Z1_INI);
        register_in(&mut ini, &reg());
        unregister_in(&mut ini);
        assert!(!ini.has_section(SECTION));
        assert_eq!(ini.indexed_key_for(BVS_INDEX, SECTION), None);
        // fremde Einträge unberührt
        assert_eq!(ini.get(BVS_INDEX, "NAME1"), Some("CONVIS_PRAXISARCHIV"));
    }

    #[test]
    fn unregister_raeumt_legacy_systems_eintrag() {
        let mut ini = Ini::parse("[SYSTEMS]\r\n1=PRAXISHUB\r\n[PRAXISHUB]\r\nName=alt\r\n");
        unregister_in(&mut ini);
        assert!(!ini.has_section(SECTION));
        assert_eq!(ini.indexed_key_for(LEGACY_SYSTEMS, SECTION), None);
    }

    #[test]
    fn default_path_respektiert_env() {
        std::env::set_var("VDDS_INI", "/tmp/custom.ini");
        assert_eq!(default_ini_path(), PathBuf::from("/tmp/custom.ini"));
        std::env::remove_var("VDDS_INI");
    }
}
