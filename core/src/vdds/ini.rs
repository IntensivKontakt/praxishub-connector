//! `VDDS_MMI.INI` — lokale Selbst-Registrierung als BVS-Modul.
//!
//! Laut VDDS-Konzept trägt sich jede teilnehmende Software lokal in die
//! `VDDS_MMI.INI` ein (freier Wettbewerb, keine Whitelist). Der PVS ruft dann
//! generisch alle registrierten Module auf.
//!
//! Die Datei ist **Windows-1252**-kodiert und liegt traditionell im Windows-
//! Verzeichnis (`%WINDIR%\VDDS_MMI.INI`); Pfad per Env `VDDS_INI` überschreibbar.
//!
//! Schema unten ist an einer **echten Z1.PRO/CGM-`VDDS_MMI.INI`** (Pilot, PRA-15
//! Prüfpunkt 1) ausgerichtet: BVS-Module werden im `[BVS]`-Index über `NAMEn`
//! gelistet, die Modul-Sektion trägt `PATDATIMPORT` (vom PVS mit Patientenkontext
//! aufgerufenes Programm), `STAGES` und `VERSION` (VDDS-media-Schnittstellenversion).

use crate::error::Result;
use encoding_rs::WINDOWS_1252;
use std::path::{Path, PathBuf};

/// Abschnittsname unseres Moduls (freie Namen-ID).
pub const SECTION: &str = "PRAXISHUB";
/// Index-Abschnitt, in dem Z1 alle teilnehmenden BVS-Module listet (`NAME1`, `NAME2`, …).
const BVS_INDEX: &str = "BVS";
/// VDDS-media-Schnittstellenversion, die wir registrieren (so steht es real in der Z1-INI).
const VDDS_VERSION: &str = "1.3";
/// Unterstützte media-Stufen: 1 (Basis) + 6 (BVS→PVS-Push, Auto-Import via `MMOINFIMPORT`).
const STAGES: &str = "16";

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

// ── reine In-Memory-Logik (unit-testbar) ─────────────────────────────────────

pub fn register_in(ini: &mut Ini, reg: &VddsRegistration) {
    // Im [BVS]-Index unter der nächsten freien NAMEn-Nummer registrieren, falls
    // noch nicht da (Z1 listet BVS-Module so: NAME1=…, NAME2=…).
    if ini.bvs_index_of(SECTION).is_none() {
        let next = ini.next_bvs_index();
        ini.set(BVS_INDEX, &format!("NAME{next}"), SECTION);
    }
    let prog = reg.program_path.to_string_lossy();
    let dir = reg.install_dir.to_string_lossy();
    ini.set(SECTION, "NAME", "Praxishub Connector");
    // Programm, das der PVS mit dem Patientenkontext (Austausch-INI) aufruft.
    ini.set(SECTION, "PATDATIMPORT", &prog);
    ini.set(SECTION, "PATDATIMPORT_OS", "1");
    ini.set(SECTION, "PATDATIMPORT_EVENT", "");
    ini.set(SECTION, "PFAD", &dir);
    ini.set(SECTION, "STAGES", STAGES);
    ini.set(SECTION, "VERSION", VDDS_VERSION);
}

pub fn unregister_in(ini: &mut Ini) {
    ini.remove_bvs_index(SECTION);
    ini.remove_section(SECTION);
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

    /// NAMEn-Nummer im [BVS]-Index, deren Wert == `value` (z. B. `NAME3=PRAXISHUB` → 3).
    fn bvs_index_of(&self, value: &str) -> Option<u32> {
        let sec = self.sections.iter().find(|s| s.name.eq_ignore_ascii_case(BVS_INDEX))?;
        sec.entries
            .iter()
            .find(|(_, v)| v.eq_ignore_ascii_case(value))
            .and_then(|(k, _)| bvs_name_number(k))
    }

    fn next_bvs_index(&self) -> u32 {
        self.sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(BVS_INDEX))
            .map(|sec| {
                sec.entries
                    .iter()
                    .filter_map(|(k, _)| bvs_name_number(k))
                    .max()
                    .map(|m| m + 1)
                    .unwrap_or(1)
            })
            .unwrap_or(1)
    }

    fn remove_bvs_index(&mut self, value: &str) {
        if let Some(sec) = self.sections.iter_mut().find(|s| s.name.eq_ignore_ascii_case(BVS_INDEX)) {
            sec.entries.retain(|(_, v)| !v.eq_ignore_ascii_case(value));
        }
    }
}

/// Parst einen `[BVS]`-Index-Schlüssel wie `NAME3` zur Nummer `3`. Andere
/// Schlüssel (z. B. `ARCHIV`) liefern `None` und werden so ignoriert.
fn bvs_name_number(key: &str) -> Option<u32> {
    key.to_ascii_uppercase().strip_prefix("NAME").and_then(|n| n.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> VddsRegistration {
        VddsRegistration {
            program_path: PathBuf::from("C:\\Apps\\praxishub-connector.exe"),
            install_dir: PathBuf::from("C:\\Apps"),
        }
    }

    #[test]
    fn roundtrip_erhaelt_eintraege() {
        let text = "[BVS]\r\nNAME1=FOO\r\n[FOO]\r\nNAME=Foo\r\n";
        let ini = Ini::parse(text);
        assert_eq!(ini.get("FOO", "NAME"), Some("Foo"));
        assert_eq!(ini.get("BVS", "NAME1"), Some("FOO"));
    }

    #[test]
    fn register_fuegt_index_und_section_hinzu() {
        // Vorhandene BVS-Module wie in einer echten Z1-INI.
        let mut ini = Ini::parse("[BVS]\r\nARCHIV=BVS_ARCHIV\r\nNAME1=FOO\r\nNAME2=BAR\r\n");
        register_in(&mut ini, &reg());
        assert!(ini.has_section(SECTION));
        // Registriert sich im [BVS]-Index unter der nächsten freien NAMEn-Nummer.
        assert_eq!(ini.bvs_index_of(SECTION), Some(3));
        assert_eq!(ini.get("BVS", "NAME3"), Some(SECTION));
        // Modul-Sektion trägt das vom PVS aufgerufene Programm + Stufen/Version.
        assert_eq!(ini.get(SECTION, "PATDATIMPORT"), Some("C:\\Apps\\praxishub-connector.exe"));
        assert_eq!(ini.get(SECTION, "STAGES"), Some("16"));
        assert_eq!(ini.get(SECTION, "VERSION"), Some("1.3"));
    }

    #[test]
    fn register_ohne_bvs_index_startet_bei_name1() {
        let mut ini = Ini::default();
        register_in(&mut ini, &reg());
        assert_eq!(ini.get("BVS", "NAME1"), Some(SECTION));
    }

    #[test]
    fn register_ist_idempotent() {
        let mut ini = Ini::default();
        register_in(&mut ini, &reg());
        register_in(&mut ini, &reg());
        // genau ein Index-Eintrag für PRAXISHUB
        let count = Ini::parse(&ini.to_text())
            .sections
            .iter()
            .find(|s| s.name == BVS_INDEX)
            .map(|s| s.entries.iter().filter(|(_, v)| v == SECTION).count())
            .unwrap_or(0);
        assert_eq!(count, 1);
    }

    #[test]
    fn unregister_entfernt_alles() {
        let mut ini = Ini::default();
        register_in(&mut ini, &reg());
        unregister_in(&mut ini);
        assert!(!ini.has_section(SECTION));
        assert_eq!(ini.bvs_index_of(SECTION), None);
    }

    #[test]
    fn default_path_respektiert_env() {
        std::env::set_var("VDDS_INI", "/tmp/custom.ini");
        assert_eq!(default_ini_path(), PathBuf::from("/tmp/custom.ini"));
        std::env::remove_var("VDDS_INI");
    }
}
