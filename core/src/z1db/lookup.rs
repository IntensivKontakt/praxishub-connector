//! Patienten-Auflösung Name + Geburtsdatum → Z1-`PATNR`, direkt über die
//! `PAT`-Tabelle (indizierte Suche). Ersetzt den fragilen PraxisArchiv-COM-Lookup.

use crate::error::Result;
use crate::matching::PatientKey;
use crate::z1db::client::Z1Connection;

/// Wandelt ein beliebiges Datumsformat in Z1s `JJJJMMTT` (nur Ziffern) um.
/// Erkennt `TT.MM.JJJJ` (bzw. `TTMMJJJJ`) und `JJJJMMTT`.
fn to_z1_date(s: &str) -> String {
    let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() == 8 && !(d.starts_with("19") || d.starts_with("20")) {
        // vermutlich TTMMJJJJ → JJJJMMTT
        format!("{}{}{}", &d[4..8], &d[2..4], &d[0..2])
    } else {
        d
    }
}

/// Löst genau einen Patienten auf. Rückgabe:
///   * `Some(patnr)`  — eindeutiger Treffer
///   * `None`         — kein oder mehrdeutiger Treffer (dann NICHT schreiben)
///
/// Kandidaten werden per Geburtsdatum vorselektiert (indiziert genug, klein) und
/// anschließend über [`PatientKey`] (normalisiert, format-tolerant) verglichen —
/// Vorname ist Pflicht, damit Zwillinge nicht vertauscht werden.
pub async fn resolve_patnr(
    conn: &mut Z1Connection,
    last_name: &str,
    first_name: &str,
    birth_date: &str,
) -> Result<Option<String>> {
    if last_name.trim().is_empty() || first_name.trim().is_empty() || birth_date.trim().is_empty() {
        return Ok(None);
    }
    let dob_z1 = to_z1_date(birth_date);
    let rows = conn
        .rows(
            "SELECT LTRIM(RTRIM(PATNR)) AS PATNR, ISNULL(PATNAME,'') AS PATNAME, \
             ISNULL(PATVORNAME,'') AS PATVORNAME FROM PAT \
             WHERE GEBDATUM = @P1 AND ISNULL(VERSTORBENAM,'') = '' AND ISNULL(GESPERRT,'') = ''",
            &[&dob_z1],
        )
        .await?;

    let want = PatientKey::new(last_name, first_name, birth_date);
    let mut hits: Vec<String> = Vec::new();
    for row in &rows {
        let patnr = row.get::<&str, _>("PATNR").unwrap_or("").trim().to_string();
        let name = row.get::<&str, _>("PATNAME").unwrap_or("");
        let vorname = row.get::<&str, _>("PATVORNAME").unwrap_or("");
        if PatientKey::new(name, vorname, &dob_z1).matches(&want) {
            hits.push(patnr);
        }
    }
    hits.dedup();
    match hits.len() {
        1 => Ok(Some(hits.remove(0))),
        _ => Ok(None), // 0 = unbekannt, >1 = mehrdeutig → sicherheitshalber nicht schreiben
    }
}
