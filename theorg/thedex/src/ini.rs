//! INI-Syntax einer THEDEX-Datei (Abschnitt 2.4 der Spezifikation).
//!
//! Absichtlich tolerant: `\r\n` und `\n`, Leerraum um Schlüssel und Wert,
//! Kommentarzeilen (`;`, `#`) und Zeilen ohne `=` werden überlesen. Schlüssel
//! und Abschnittsnamen behalten ihre Schreibweise; verglichen wird ohne
//! Rücksicht auf Groß-/Kleinschreibung.

use serde::Serialize;

/// Ein Datenelement `Schlüssel=Wert`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Feld {
    pub schluessel: String,
    pub wert: String,
}

impl Feld {
    pub fn neu(schluessel: impl Into<String>, wert: impl Into<String>) -> Self {
        Feld {
            schluessel: schluessel.into(),
            wert: wert.into(),
        }
    }
}

/// Ein Abschnitt `[Name]` mit seinen Datenelementen in Dateireihenfolge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Abschnitt {
    pub name: String,
    pub felder: Vec<Feld>,
}

impl Abschnitt {
    pub fn neu(name: impl Into<String>) -> Self {
        Abschnitt {
            name: name.into(),
            felder: Vec::new(),
        }
    }

    /// Erster Wert zum Schlüssel (Groß-/Kleinschreibung egal).
    pub fn wert(&self, schluessel: &str) -> Option<&str> {
        self.felder
            .iter()
            .find(|f| f.schluessel.eq_ignore_ascii_case(schluessel))
            .map(|f| f.wert.as_str())
    }
}

/// Zerlegt Text in Abschnitte. Zeilen vor dem ersten Abschnitt werden
/// verworfen; ihre Anzahl steht in `verworfen`.
#[derive(Debug, Default)]
pub struct Zerlegt {
    pub abschnitte: Vec<Abschnitt>,
    pub verworfen: usize,
}

pub fn zerlegen(text: &str) -> Zerlegt {
    let mut z = Zerlegt::default();
    for roh in text.split('\n') {
        let zeile = roh.trim_matches(|c: char| c == '\r' || c.is_whitespace() || c == '\u{FEFF}');
        if zeile.is_empty() || zeile.starts_with(';') || zeile.starts_with('#') {
            continue;
        }
        if zeile.starts_with('[') {
            if let Some(ende) = zeile.find(']') {
                z.abschnitte.push(Abschnitt::neu(zeile[1..ende].trim()));
                continue;
            }
        }
        match (zeile.split_once('='), z.abschnitte.last_mut()) {
            (Some((k, v)), Some(abschnitt)) if !k.trim().is_empty() => {
                abschnitt.felder.push(Feld::neu(k.trim(), v.trim()));
            }
            _ => z.verworfen += 1,
        }
    }
    z
}

/// Schreibt Abschnitte als Text mit Windows-Zeilenenden und einer Leerzeile
/// zwischen den Abschnitten (wie in den Beispielen der Spezifikation).
pub fn schreiben(abschnitte: &[Abschnitt]) -> String {
    let mut out = String::new();
    for (i, a) in abschnitte.iter().enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        out.push('[');
        out.push_str(&a.name);
        out.push_str("]\r\n");
        for f in &a.felder {
            out.push_str(&einzeilig(&f.schluessel));
            out.push('=');
            out.push_str(&einzeilig(&f.wert));
            out.push_str("\r\n");
        }
    }
    out
}

/// Ein Wert darf keinen Zeilenumbruch enthalten; Umbrüche werden zu Leerzeichen.
fn einzeilig(s: &str) -> String {
    if s.contains(['\r', '\n']) {
        s.replace("\r\n", " ").replace(['\r', '\n'], " ")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerant_zerlegen() {
        let z = zerlegen("vorher\n[Thedex]\r\n ThedexVersionNumber = 2.1 \r\n\r\n; Kommentar\n[client]\nCLastName=Muster=mann\nohneGleich\nleer=\n");
        assert_eq!(z.verworfen, 2);
        assert_eq!(z.abschnitte.len(), 2);
        assert_eq!(z.abschnitte[0].wert("thedexversionnumber"), Some("2.1"));
        assert_eq!(z.abschnitte[1].name, "client");
        assert_eq!(z.abschnitte[1].wert("CLASTNAME"), Some("Muster=mann"));
        assert_eq!(z.abschnitte[1].wert("leer"), Some(""));
    }

    #[test]
    fn schreiben_crlf_ohne_umbruch_im_wert() {
        let mut a = Abschnitt::neu("Client");
        a.felder.push(Feld::neu("CStreet", "Zeile1\nZeile2"));
        assert_eq!(schreiben(&[a]), "[Client]\r\nCStreet=Zeile1 Zeile2\r\n");
    }
}
