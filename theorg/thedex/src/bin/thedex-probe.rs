//! `thedex-probe` – Sichtung eines THEDEX-Ordners in der Fernwartung.
//!
//! Liest nur: listet die Provider aus `THEDEX.INI`, parst alle `.TDX` im
//! angegebenen Unterordner und gibt die abgebildeten Connector-v2-Objekte als
//! JSON auf stdout aus. Löscht, verschiebt und schreibt nichts.
//!
//! ```text
//! thedex-probe <THEDEX-Ordner> [Unterordner] [--pvs-provider THEORG] [--roh]
//! ```
//!
//! `Unterordner` ist der Eingang, der gelesen wird (Standard `PRAXHUB`).
//! `--roh` gibt zusätzlich das gelesene Datenmodell jeder Datei aus.

use praxishub_thedex::verzeichnis::{ini_finden, ordner_lesen, providers_lesen};
use praxishub_thedex::{abbilden, AbbildungsOptionen, Eingang, PROVIDER_STANDARD};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

const HILFE: &str = "thedex-probe <THEDEX-Ordner> [Unterordner] [--pvs-provider THEORG] [--roh]\n\
Liest einen THEDEX-Ordner nur lesend und gibt Provider und abgebildete v2-Objekte als JSON aus.";

fn main() -> ExitCode {
    let mut positional: Vec<String> = Vec::new();
    let mut opt = AbbildungsOptionen::default();
    let mut roh = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{HILFE}");
                return ExitCode::SUCCESS;
            }
            "--roh" => roh = true,
            "--pvs-provider" => match args.next() {
                Some(p) => opt.pvs_provider = p,
                None => return nutzung("--pvs-provider braucht einen Wert"),
            },
            _ if a.starts_with("--") => return nutzung(&format!("unbekannte Option {a}")),
            _ => positional.push(a),
        }
    }
    let (wurzel, unterordner) = match positional.as_slice() {
        [w] => (PathBuf::from(w), PROVIDER_STANDARD.to_string()),
        [w, u] => (PathBuf::from(w), u.clone()),
        _ => return nutzung("THEDEX-Ordner fehlt"),
    };

    let providers = match providers_lesen(&wurzel) {
        Ok(p) => json!(p),
        Err(e) => json!({ "fehler": e.to_string() }),
    };
    let ordner = wurzel.join(&unterordner);
    let mut ausgabe = json!({
        "thedex_ordner": wurzel.display().to_string(),
        "ini": ini_finden(&wurzel).map(|p| p.display().to_string()),
        "providers": providers,
        "unterordner": ordner.display().to_string(),
        "pvs_provider": opt.pvs_provider,
    });

    let eingaenge = match ordner_lesen(&ordner, Duration::ZERO) {
        Ok(e) => e,
        Err(e) => {
            ausgabe["fehler"] = json!(e.to_string());
            drucken(&ausgabe);
            return ExitCode::FAILURE;
        }
    };

    let mut summe = Summe::default();
    let mut dateien: Vec<Value> = Vec::new();
    for e in eingaenge {
        summe.dateien += 1;
        match e {
            Eingang::Defekt { pfad, fehler } => {
                summe.defekt += 1;
                dateien.push(json!({ "datei": dateiname(&pfad), "fehler": fehler }));
            }
            Eingang::Lesbar(d) => {
                let n = &d.nachricht;
                let ab = abbilden(n, &opt);
                summe.patienten += ab.patienten.len();
                summe.mitgliedschaften += ab.mitgliedschaften.len();
                summe.checkins += ab.checkins.len();
                let mut eintrag = json!({
                    "datei": dateiname(&d.pfad),
                    "typ": n.typ().als_text(),
                    "sender": n.sender.as_ref().and_then(|s| s.identification.clone()),
                    "zeichensatz": n.zeichensatz().kennung(),
                    "nachricht_vom": [n.message.date.clone(), n.message.time.clone()],
                    "anhang": d.anhang.as_ref().map(|a| dateiname(a)),
                    "abbildung": ab,
                });
                if roh {
                    eintrag["roh"] = json!(n);
                }
                dateien.push(eintrag);
            }
        }
    }
    ausgabe["dateien"] = json!(dateien);
    ausgabe["summe"] = json!({
        "dateien": summe.dateien,
        "defekt": summe.defekt,
        "patienten": summe.patienten,
        "mitgliedschaften": summe.mitgliedschaften,
        "checkins": summe.checkins,
    });
    drucken(&ausgabe);
    ExitCode::SUCCESS
}

#[derive(Default)]
struct Summe {
    dateien: usize,
    defekt: usize,
    patienten: usize,
    mitgliedschaften: usize,
    checkins: usize,
}

fn dateiname(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn drucken(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|e| format!("{{\"fehler\": \"{e}\"}}"))
    );
}

fn nutzung(meldung: &str) -> ExitCode {
    eprintln!("{meldung}\n\n{HILFE}");
    ExitCode::from(2)
}
