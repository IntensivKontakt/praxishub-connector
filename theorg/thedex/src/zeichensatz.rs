//! Zeichensätze einer THEDEX-Datei.
//!
//! `MessageCodePageInformation` kennt zwei Werte: `ANSI` (Windows-1252) und
//! `OEM` (DOS, in Deutschland Codepage 850). Windows-1252 kommt aus
//! `encoding_rs`; CP850 kennt `encoding_rs` nicht (nur WHATWG-Kodierungen),
//! daher steht die obere Hälfte der Tabelle hier fest im Code.
//!
//! Beide Zeichensätze sind 8-Bit und ASCII-kompatibel. Dekodieren ist
//! verlustfrei (jedes Byte hat ein Zeichen); beim Kodieren wird ein Zeichen,
//! das der Zeichensatz nicht kennt, durch `?` ersetzt.

use serde::Serialize;
use std::sync::OnceLock;

/// Zeichensatz laut `MessageCodePageInformation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum Zeichensatz {
    /// `ANSI` = Windows-1252 (Standard, wenn nichts angegeben ist).
    #[default]
    Ansi,
    /// `OEM` = Codepage 850 (MS-DOS Westeuropa).
    Oem,
}

impl Zeichensatz {
    /// Liest den Wert aus `MessageCodePageInformation` (Groß-/Kleinschreibung
    /// egal). Unbekannte oder leere Werte ergeben ANSI.
    pub fn aus_kennung(wert: &str) -> Self {
        let w = wert.trim();
        if w.eq_ignore_ascii_case("OEM") || w == "850" || w == "437" {
            Zeichensatz::Oem
        } else {
            Zeichensatz::Ansi
        }
    }

    /// Schreibweise in der Datei.
    pub fn kennung(self) -> &'static str {
        match self {
            Zeichensatz::Ansi => "ANSI",
            Zeichensatz::Oem => "OEM",
        }
    }

    fn obere_haelfte(self) -> &'static [char; 128] {
        match self {
            Zeichensatz::Ansi => cp1252_tabelle(),
            Zeichensatz::Oem => &CP850_OBEN,
        }
    }

    /// Bytes → Text. Verlustfrei, jedes Byte ergibt genau ein Zeichen.
    pub fn dekodieren(self, bytes: &[u8]) -> String {
        let tabelle = self.obere_haelfte();
        bytes
            .iter()
            .map(|&b| {
                if b < 0x80 {
                    b as char
                } else {
                    tabelle[(b - 0x80) as usize]
                }
            })
            .collect()
    }

    /// Text → Bytes. Zeichen ohne Entsprechung werden zu `?`.
    pub fn kodieren(self, text: &str) -> Vec<u8> {
        let tabelle = self.obere_haelfte();
        text.chars()
            .map(|c| {
                if (c as u32) < 0x80 {
                    c as u8
                } else {
                    tabelle
                        .iter()
                        .position(|&t| t == c)
                        .map(|i| 0x80 + i as u8)
                        .unwrap_or(b'?')
                }
            })
            .collect()
    }

    /// `true`, wenn jedes Zeichen des Textes im Zeichensatz darstellbar ist.
    pub fn darstellbar(self, text: &str) -> bool {
        let tabelle = self.obere_haelfte();
        text.chars()
            .all(|c| (c as u32) < 0x80 || tabelle.contains(&c))
    }
}

/// Obere Hälfte von Windows-1252, einmalig aus `encoding_rs` gewonnen. WHATWG
/// belegt auch die fünf in Microsofts Tabelle undefinierten Bytes (0x81, 0x8D,
/// 0x8F, 0x90, 0x9D) mit den C1-Steuerzeichen; damit bleibt jedes Byte erhalten.
fn cp1252_tabelle() -> &'static [char; 128] {
    static TABELLE: OnceLock<[char; 128]> = OnceLock::new();
    TABELLE.get_or_init(|| {
        let mut t = ['?'; 128];
        for (i, slot) in t.iter_mut().enumerate() {
            let byte = [0x80u8 + i as u8];
            let (text, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(&byte);
            *slot = text.chars().next().unwrap_or('?');
        }
        t
    })
}

/// Codepage 850, Bytes 0x80–0xFF.
const CP850_OBEN: [char; 128] = [
    '\u{00C7}', '\u{00FC}', '\u{00E9}', '\u{00E2}', '\u{00E4}', '\u{00E0}', '\u{00E5}', '\u{00E7}',
    '\u{00EA}', '\u{00EB}', '\u{00E8}', '\u{00EF}', '\u{00EE}', '\u{00EC}', '\u{00C4}', '\u{00C5}',
    '\u{00C9}', '\u{00E6}', '\u{00C6}', '\u{00F4}', '\u{00F6}', '\u{00F2}', '\u{00FB}', '\u{00F9}',
    '\u{00FF}', '\u{00D6}', '\u{00DC}', '\u{00F8}', '\u{00A3}', '\u{00D8}', '\u{00D7}', '\u{0192}',
    '\u{00E1}', '\u{00ED}', '\u{00F3}', '\u{00FA}', '\u{00F1}', '\u{00D1}', '\u{00AA}', '\u{00BA}',
    '\u{00BF}', '\u{00AE}', '\u{00AC}', '\u{00BD}', '\u{00BC}', '\u{00A1}', '\u{00AB}', '\u{00BB}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{00C1}', '\u{00C2}', '\u{00C0}',
    '\u{00A9}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255D}', '\u{00A2}', '\u{00A5}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2500}', '\u{253C}', '\u{00E3}', '\u{00C3}',
    '\u{255A}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256C}', '\u{00A4}',
    '\u{00F0}', '\u{00D0}', '\u{00CA}', '\u{00CB}', '\u{00C8}', '\u{0131}', '\u{00CD}', '\u{00CE}',
    '\u{00CF}', '\u{2518}', '\u{250C}', '\u{2588}', '\u{2584}', '\u{00A6}', '\u{00CC}', '\u{2580}',
    '\u{00D3}', '\u{00DF}', '\u{00D4}', '\u{00D2}', '\u{00F5}', '\u{00D5}', '\u{00B5}', '\u{00FE}',
    '\u{00DE}', '\u{00DA}', '\u{00DB}', '\u{00D9}', '\u{00FD}', '\u{00DD}', '\u{00AF}', '\u{00B4}',
    '\u{00AD}', '\u{00B1}', '\u{2017}', '\u{00BE}', '\u{00B6}', '\u{00A7}', '\u{00F7}', '\u{00B8}',
    '\u{00B0}', '\u{00A8}', '\u{00B7}', '\u{00B9}', '\u{00B3}', '\u{00B2}', '\u{25A0}', '\u{00A0}',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn umlaute_ansi() {
        let bytes = Zeichensatz::Ansi.kodieren("Müller Straße Ä");
        assert_eq!(bytes, b"M\xFCller Stra\xDFe \xC4");
        assert_eq!(Zeichensatz::Ansi.dekodieren(&bytes), "Müller Straße Ä");
    }

    #[test]
    fn umlaute_oem() {
        let bytes = Zeichensatz::Oem.kodieren("Müller Straße Ä ö");
        assert_eq!(bytes, b"M\x81ller Stra\xE1e \x8E \x94");
        assert_eq!(Zeichensatz::Oem.dekodieren(&bytes), "Müller Straße Ä ö");
    }

    #[test]
    fn jedes_byte_ueberlebt_die_rundreise() {
        let alle: Vec<u8> = (0u8..=255).collect();
        for z in [Zeichensatz::Ansi, Zeichensatz::Oem] {
            assert_eq!(z.kodieren(&z.dekodieren(&alle)), alle, "{z:?}");
        }
    }

    #[test]
    fn nicht_darstellbar_wird_fragezeichen() {
        assert_eq!(Zeichensatz::Ansi.kodieren("a\u{4E2D}b"), b"a?b");
        assert!(!Zeichensatz::Oem.darstellbar("€"));
        assert!(Zeichensatz::Ansi.darstellbar("€"));
    }

    #[test]
    fn kennung() {
        assert_eq!(Zeichensatz::aus_kennung(" oem "), Zeichensatz::Oem);
        assert_eq!(Zeichensatz::aus_kennung("ANSI"), Zeichensatz::Ansi);
        assert_eq!(Zeichensatz::aus_kennung(""), Zeichensatz::Ansi);
    }
}
