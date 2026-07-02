//! Weg A: Auflösung der Z1-`PatientenID` aus **Name + Vorname + Geburtsdatum**
//! über die PraxisArchiv-Datenbank — für Dokumente, deren PatientenID das Backend
//! (noch) nicht kennt (typisch: Doctolib-Neupatienten, die erst in der Praxis eine
//! Z1-Nummer bekommen, sobald die Karte „steckt").
//!
//! Die PraxisArchiv-DB liegt hinter einem **32-bit-In-Process-COM-Server**
//! (`DBClient.dll`). Der 64-bit-Connector kann diesen nicht direkt laden, darum
//! wird die Abfrage in einem kurzlebigen **32-bit-PowerShell-Prozess** ausgeführt
//! (`SysWOW64\WindowsPowerShell`), der das eingebettete Skript [`pa_lookup.ps1`]
//! per `-EncodedCommand` erhält. Das Skript spricht `IDBHandler → IDBServer →
//! ITables.PerformCountSQL` **rein lesend** an (COUNT-then-fetch, Details dort und
//! in `docs/praxisarchiv-com.md`). Kein Login/Passwort nötig — `Connect()` läuft im
//! Kontext des angemeldeten, PraxisArchiv-berechtigten Praxis-Nutzers.
//!
//! Das Verfahren ist bewusst konservativ: Nur ein **eindeutiger** Treffer liefert
//! eine PatientenID; bei 0 oder mehreren (auch nach dem PLZ-Tiebreaker) wird
//! nichts abgelegt — lieber gar nicht als falsch.

/// Ergebnis eines PatientenID-Lookups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatientLookup {
    /// Genau ein Treffer — die aufgelöste Z1-`PatientenID`.
    Found(String),
    /// Kein Treffer (Patient (noch) nicht angelegt) — später erneut versuchen.
    NotFound,
    /// Mehrere gleichwertige Treffer, auch nach Tiebreaker — bewusst nicht ablegen.
    Ambiguous,
    /// Lookup konnte nicht durchgeführt werden (kein Windows, PraxisArchiv nicht
    /// erreichbar, unerwartete Ausgabe). Wie `NotFound` behandeln (Retry), aber
    /// die Ursache ist eine andere.
    Unavailable(String),
}

/// Das read-only-Lookup-Skript, zur Compile-Zeit eingebettet (kein separates
/// Bundling nötig). Wird zur Laufzeit an die 32-bit-PowerShell übergeben.
#[cfg(windows)]
const LOOKUP_PS1: &str = include_str!("pa_lookup.ps1");

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct LookupJson {
    status: String,
    patient_id: Option<String>,
    message: Option<String>,
}

/// Löst die PatientenID auf. `dob` im Backend-Format (`JJJJMMTT`) oder einem der
/// von [`crate::matching::normalize_birthdate`] akzeptierten Formate — wird intern
/// ins von PraxisArchiv erwartete `TT.MM.JJJJ` gebracht. `zip` optional (Tiebreaker
/// bei Namensvettern); leer lassen, wenn unbekannt.
pub fn resolve_patient_id(last: &str, first: &str, dob: &str, zip: &str) -> PatientLookup {
    // PraxisArchiv vergleicht die Geburtsdatum-Spalte als `TT.MM.JJJJ`; ISO/kompakt
    // würde einen DB-Konvertierungsfehler auslösen. Zentral über die Match-
    // Normalisierung (JJJJMMTT) und dann in deutsches Format umsetzen.
    let dob_de = match crate::matching::normalize_birthdate(dob) {
        Some(ymd) if ymd.len() == 8 => {
            format!("{}.{}.{}", &ymd[6..8], &ymd[4..6], &ymd[0..4])
        }
        _ => return PatientLookup::Unavailable(format!("unbrauchbares Geburtsdatum: {dob:?}")),
    };

    #[cfg(not(windows))]
    {
        let _ = (last, first, zip, dob_de);
        PatientLookup::Unavailable("PatientenID-Lookup nur unter Windows".into())
    }
    #[cfg(windows)]
    {
        run_lookup(last, first, &dob_de, zip)
    }
}

#[cfg(windows)]
fn run_lookup(last: &str, first: &str, dob_de: &str, zip: &str) -> PatientLookup {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::process::Command;

    // `-EncodedCommand` erwartet UTF-16LE-Base64. Das umgeht zugleich die
    // ExecutionPolicy (kein Skript-Datei-Aufruf) — wichtig in verwalteten
    // Praxis-Umgebungen.
    let utf16: Vec<u8> = LOOKUP_PS1.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = STANDARD.encode(&utf16);

    // Explizit die 32-bit-PowerShell (SysWOW64) — nur sie lädt den 32-bit-COM-
    // Server. `%SystemRoot%` statt hartem C:, falls Windows woanders liegt.
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let powershell = format!(
        "{system_root}\\SysWOW64\\WindowsPowerShell\\v1.0\\powershell.exe"
    );

    let output = Command::new(&powershell)
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        // Eingaben über die Umgebung (nicht die Kommandozeile) → kein Leak in die
        // Prozessliste; das Skript escaped sie zusätzlich fürs SQL.
        .env("PA_LAST", last)
        .env("PA_FIRST", first)
        .env("PA_DOB", dob_de)
        .env("PA_ZIP", zip)
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => return PatientLookup::Unavailable(format!("PowerShell nicht startbar: {e}")),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Das Skript gibt genau eine JSON-Zeile aus; robust die letzte nicht-leere nehmen.
    let line = stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");

    match serde_json::from_str::<LookupJson>(line) {
        Ok(j) => match j.status.as_str() {
            "found" => match j.patient_id {
                Some(id) if !id.trim().is_empty() => PatientLookup::Found(id.trim().to_string()),
                _ => PatientLookup::Unavailable("Treffer ohne PatientenID".into()),
            },
            "none" => PatientLookup::NotFound,
            "ambiguous" => PatientLookup::Ambiguous,
            _ => PatientLookup::Unavailable(
                j.message.unwrap_or_else(|| "Lookup fehlgeschlagen".into()),
            ),
        },
        Err(e) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            PatientLookup::Unavailable(format!(
                "unerwartete Lookup-Ausgabe ({e}): stdout={line:?} stderr={:?}",
                stderr.trim()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ungueltiges_geburtsdatum_ist_unavailable() {
        assert!(matches!(
            resolve_patient_id("Groth", "Nikolas", "keinDatum", ""),
            PatientLookup::Unavailable(_)
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn ausserhalb_windows_unavailable() {
        assert!(matches!(
            resolve_patient_id("Groth", "Nikolas", "20010223", ""),
            PatientLookup::Unavailable(_)
        ));
    }
}
