//! Datenformate der Spezifikation (Abschnitt 2.4 „Datenformate“).
//!
//! Datum `TTMMJJJJ`, Uhrzeit `HH:MM`. Beim Lesen wird zusätzlich
//! `TT.MM.JJJJ`, `JJJJ-MM-TT` und `HH:MM:SS` angenommen.

use chrono::{NaiveDate, NaiveTime};

/// `TTMMJJJJ` (auch `TT.MM.JJJJ` oder `JJJJ-MM-TT`) → Datum. Leer/ungültig → `None`.
pub fn datum_lesen(wert: &str) -> Option<NaiveDate> {
    let w = wert.trim();
    if w.is_empty() {
        return None;
    }
    if w.len() == 8 && w.bytes().all(|b| b.is_ascii_digit()) {
        let tag = w[0..2].parse().ok()?;
        let monat = w[2..4].parse().ok()?;
        let jahr = w[4..8].parse().ok()?;
        return NaiveDate::from_ymd_opt(jahr, monat, tag);
    }
    NaiveDate::parse_from_str(w, "%d.%m.%Y")
        .or_else(|_| NaiveDate::parse_from_str(w, "%Y-%m-%d"))
        .ok()
}

/// Datum → `TTMMJJJJ`.
pub fn datum_schreiben(d: NaiveDate) -> String {
    d.format("%d%m%Y").to_string()
}

/// `HH:MM` (auch `H:MM`, `HH:MM:SS`) → Uhrzeit. Leer/ungültig → `None`.
pub fn zeit_lesen(wert: &str) -> Option<NaiveTime> {
    let w = wert.trim();
    if w.is_empty() {
        return None;
    }
    NaiveTime::parse_from_str(w, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(w, "%H:%M:%S"))
        .ok()
}

/// Uhrzeit → `HH:MM`.
pub fn zeit_schreiben(t: NaiveTime) -> String {
    t.format("%H:%M").to_string()
}

/// Leere oder nur aus Leerraum bestehende Werte zählen als „nicht gesetzt“.
pub fn gefuellt(wert: &Option<String>) -> Option<&str> {
    wert.as_deref().map(str::trim).filter(|w| !w.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datum() {
        let d = NaiveDate::from_ymd_opt(2008, 3, 12).unwrap();
        assert_eq!(datum_lesen("12032008"), Some(d));
        assert_eq!(datum_lesen("12.03.2008"), Some(d));
        assert_eq!(datum_lesen("2008-03-12"), Some(d));
        assert_eq!(datum_lesen("31022008"), None);
        assert_eq!(datum_lesen(""), None);
        assert_eq!(datum_schreiben(d), "12032008");
    }

    #[test]
    fn zeit() {
        let t = NaiveTime::from_hms_opt(12, 2, 0).unwrap();
        assert_eq!(zeit_lesen("12:02"), Some(t));
        assert_eq!(zeit_lesen("12:02:00"), Some(t));
        assert_eq!(zeit_lesen("9:05"), NaiveTime::from_hms_opt(9, 5, 0));
        assert_eq!(zeit_lesen("25:00"), None);
        assert_eq!(zeit_schreiben(t), "12:02");
    }
}
