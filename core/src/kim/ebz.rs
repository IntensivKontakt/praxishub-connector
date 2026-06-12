//! EBZ-Erkennung auf Mail-Header-Ebene (DSGVO-Minimierung: nur genehmigte HKPs).
//!
//! Der Connector parst den Inhalt **nicht** — er filtert nur und reicht die
//! Rohnachricht weiter. Maßgeblich ist der KIM-Header
//! `X-KIM-Dienstkennung: EBZ;ANW;2.0.0` (Antwortdatensatz der Krankenkasse).

/// Trifft die Dienstkennung auf eine EBZ-Antwort (genehmigter HKP) zu?
///
/// Versions-tolerant: matcht `EBZ;ANW` unabhängig von der Versionsendung.
pub fn is_ebz_approval(headers: &str) -> bool {
    dienstkennung(headers)
        .map(|v| {
            let v = v.to_ascii_uppercase();
            v.contains("EBZ") && v.contains("ANW")
        })
        .unwrap_or(false)
}

/// Wert des Headers `X-KIM-Dienstkennung`, getrimmt.
pub fn dienstkennung(headers: &str) -> Option<String> {
    header_value(headers, "X-KIM-Dienstkennung")
}

/// Liest den Wert eines Mail-Headers (case-insensitive, erstes Vorkommen).
/// Berücksichtigt einfache Header-Faltung (Folgezeilen mit führendem Whitespace).
pub fn header_value(headers: &str, name: &str) -> Option<String> {
    let want = name.to_ascii_lowercase();
    let mut lines = headers.lines().peekable();
    while let Some(line) = lines.next() {
        // Leerzeile trennt Header von Body.
        if line.trim().is_empty() {
            break;
        }
        if let Some((key, val)) = line.split_once(':') {
            if key.trim().to_ascii_lowercase() == want {
                let mut value = val.trim().to_string();
                // gefaltete Folgezeilen anhängen
                while let Some(next) = lines.peek() {
                    if next.starts_with([' ', '\t']) {
                        value.push(' ');
                        value.push_str(next.trim());
                        lines.next();
                    } else {
                        break;
                    }
                }
                return Some(value);
            }
        }
    }
    None
}

/// Minimale Metadaten, die der Connector mitliefert (Rest macht die Cloud).
#[derive(Debug, Clone)]
pub struct EbzSummary {
    pub dienstkennung: String,
    pub message_id: Option<String>,
    pub received_at: Option<String>,
}

pub fn summarize(raw_message: &str) -> EbzSummary {
    EbzSummary {
        dienstkennung: dienstkennung(raw_message).unwrap_or_default(),
        message_id: header_value(raw_message, "Message-ID"),
        received_at: header_value(raw_message, "Date"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPROVAL: &str = "Return-Path: <kasse@kim.telematik>\r\n\
X-KIM-Dienstkennung: EBZ;ANW;2.0.0\r\n\
Message-ID: <abc123@kim>\r\n\
Date: Thu, 12 Jun 2026 09:30:00 +0200\r\n\
Subject: Antwortdatensatz\r\n\
\r\n\
<body>";

    #[test]
    fn erkennt_genehmigten_hkp() {
        assert!(is_ebz_approval(APPROVAL));
    }

    #[test]
    fn ignoriert_fremde_dienstkennung() {
        let other = "X-KIM-Dienstkennung: ARZTBRIEF;V1\r\n\r\nbody";
        assert!(!is_ebz_approval(other));
    }

    #[test]
    fn ignoriert_ohne_header() {
        assert!(!is_ebz_approval("Subject: hi\r\n\r\nbody"));
    }

    #[test]
    fn versions_tolerant() {
        assert!(is_ebz_approval("X-KIM-Dienstkennung: EBZ;ANW;2.1.0\r\n\r\n"));
    }

    #[test]
    fn header_case_insensitive() {
        assert_eq!(
            header_value(APPROVAL, "message-id").as_deref(),
            Some("<abc123@kim>")
        );
    }

    #[test]
    fn summary_zieht_metadaten() {
        let s = summarize(APPROVAL);
        assert_eq!(s.dienstkennung, "EBZ;ANW;2.0.0");
        assert_eq!(s.message_id.as_deref(), Some("<abc123@kim>"));
        assert!(s.received_at.is_some());
    }
}
