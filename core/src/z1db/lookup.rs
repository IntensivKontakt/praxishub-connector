//! Patienten-Auflösung Name + Geburtsdatum (+ PLZ) → Z1-`PATNR`, direkt über die
//! `PAT`-Tabelle. Ersetzt den fragilen PraxisArchiv-COM-Lookup und nutzt leichtes
//! Fuzzy-Matching + PLZ-Bestätigung ([`crate::matching`]) für hohe Trefferquote —
//! bei Unsicherheit wird NIE geraten, sondern zur manuellen Zuordnung eskaliert.

use crate::error::Result;
use crate::matching::{resolve_fuzzy, Candidate, PatientKey, Resolution};
use crate::z1db::client::Z1Connection;
use std::collections::HashSet;

/// Löst einen Patienten auf:
///   * `Matched(patnr)` — sicher genug (auto-anwenden)
///   * `Review(patnrs)` — nah dran, aber unsicher/mehrdeutig → manuell ans Team
///   * `NotFound`       — niemand nah → Patient (noch) nicht in Z1 → später erneut
pub async fn resolve_patient(
    conn: &mut Z1Connection,
    last_name: &str,
    first_name: &str,
    birth_date: &str,
    zip: Option<&str>,
) -> Result<Resolution<String>> {
    let wanted = PatientKey::new(last_name, first_name, birth_date);
    if wanted.last_name.is_empty() {
        return Ok(Resolution::NotFound);
    }

    let mut cands: Vec<Candidate<String>> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // A) Kandidaten mit exaktem Geburtsdatum (der häufige Fall).
    if !wanted.birth_date.is_empty() {
        for c in fetch_by_dob(conn, &wanted.birth_date).await? {
            if seen.insert(c.payload.clone()) {
                cands.push(c);
            }
        }
    }
    // B) Fallback für Geburtsdatum-Tippfehler: gleiche PLZ + Namenspräfix.
    if let Some(z) = zip.map(str::trim).filter(|s| !s.is_empty()) {
        let prefix: String = last_name.trim().chars().take(2).collect::<String>().to_uppercase();
        if !prefix.is_empty() {
            for c in fetch_by_zip_prefix(conn, z, &prefix).await? {
                if seen.insert(c.payload.clone()) {
                    cands.push(c);
                }
            }
        }
    }

    Ok(resolve_fuzzy(&wanted, zip, &cands))
}

fn row_to_candidate(r: &tiberius::Row) -> Candidate<String> {
    let get = |c: &str| r.get::<&str, _>(c).unwrap_or("").trim().to_string();
    let patnr = get("PATNR");
    Candidate {
        key: PatientKey::new(&get("NAME"), &get("VOR"), &get("DOB")),
        zip: Some(get("PLZ")).filter(|s| !s.is_empty()),
        email: None,
        payload: patnr,
    }
}

/// Kandidaten mit exaktem Geburtsdatum (JJJJMMTT), lebend & nicht gesperrt.
async fn fetch_by_dob(conn: &mut Z1Connection, dob_z1: &str) -> Result<Vec<Candidate<String>>> {
    let rows = conn
        .rows(
            "SELECT LTRIM(RTRIM(p.PATNR)) AS PATNR, ISNULL(p.PATNAME,'') AS NAME, \
                    ISNULL(p.PATVORNAME,'') AS VOR, ISNULL(p.GEBDATUM,'') AS DOB, \
                    ISNULL(a.PLZ,'') AS PLZ \
             FROM PAT p LEFT JOIN ADR a ON LTRIM(RTRIM(a.ADRID)) = LTRIM(RTRIM(p.ADRIDP)) \
             WHERE p.GEBDATUM = @P1 AND ISNULL(p.VERSTORBENAM,'') = '' AND ISNULL(p.GESPERRT,'') = ''",
            &[&dob_z1],
        )
        .await?;
    Ok(rows.iter().map(row_to_candidate).collect())
}

/// Kandidaten mit gleicher PLZ + Nachnamen-Präfix (fängt Geburtsdatum-Tippfehler);
/// begrenzt auf 60 Zeilen (Feinvergleich erledigt das Fuzzy-Matching).
async fn fetch_by_zip_prefix(
    conn: &mut Z1Connection,
    zip: &str,
    prefix_upper: &str,
) -> Result<Vec<Candidate<String>>> {
    let like = format!("{prefix_upper}%");
    let rows = conn
        .rows(
            "SELECT TOP 60 LTRIM(RTRIM(p.PATNR)) AS PATNR, ISNULL(p.PATNAME,'') AS NAME, \
                    ISNULL(p.PATVORNAME,'') AS VOR, ISNULL(p.GEBDATUM,'') AS DOB, \
                    ISNULL(a.PLZ,'') AS PLZ \
             FROM PAT p JOIN ADR a ON LTRIM(RTRIM(a.ADRID)) = LTRIM(RTRIM(p.ADRIDP)) \
             WHERE LTRIM(RTRIM(a.PLZ)) = @P1 AND UPPER(LTRIM(p.PATNAME)) LIKE @P2 \
                   AND ISNULL(p.VERSTORBENAM,'') = '' AND ISNULL(p.GESPERRT,'') = ''",
            &[&zip, &like],
        )
        .await?;
    Ok(rows.iter().map(row_to_candidate).collect())
}
