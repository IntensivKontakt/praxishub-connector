//! THEDEX – Datei-Schnittstelle von THEORG (SOVDWAER), Spezifikation 2.1
//! vom 15.01.2015.
//!
//! Baustein des THEORG-Connectors, ohne Tauri und ohne Netzwerk:
//!
//! * [`modell`]/[`ini`]/[`zeichensatz`] – `.TDX`-Dateien lesen und schreiben
//!   (INI-Syntax, ANSI = Windows-1252 oder OEM = CP850), verlustfrei für
//!   unbekannte Felder und Abschnitte.
//! * [`verzeichnis`] – THEDEX-Ordner: Selbsteintrag in `THEDEX.INI`, eigenen
//!   Eingang lesen und erst nach Quittung löschen, defekte Dateien nach
//!   `FEHLER`, atomarer Versand in den Eingang eines anderen Providers.
//! * [`v2`] – Abbildung auf Connector-v2-Objekte (`PatientV2`,
//!   `MitgliedschaftV2`, `CheckinV2`).
//! * [`ausgang`] – Cloud-Aufträge `patient_anlegen` und `dokument_ablegen`
//!   als `ClientNew` bzw. `ClientReport`.
//!
//! Siehe `theorg/thedex/README.md`.

pub mod ausgang;
pub mod fehler;
pub mod ini;
pub mod modell;
pub mod v2;
pub mod verzeichnis;
pub mod werte;
pub mod zeichensatz;

pub use ausgang::{
    client_new, client_report, Anhang, DokumentAblegenPayload, PatientAnlegenPayload, PatientNeuV2,
    SenderKennung,
};
pub use fehler::{Ergebnis, ThedexFehler};
pub use modell::{Nachricht, Nachrichtentyp};
pub use v2::{abbilden, Abbildung, AbbildungsOptionen, CheckinV2, MitgliedschaftV2, PatientV2};
pub use verzeichnis::{Eingang, EingangsDatei, Selbsteintrag, ThedexOrdner, Verarbeitungsbericht};
pub use zeichensatz::Zeichensatz;

/// THEDEX-Version, die diese Bibliothek schreibt.
pub const THEDEX_VERSION: &str = "2.1";

/// Vorgeschlagener Providername (= Verzeichnisname, max. 8 Zeichen).
pub const PROVIDER_STANDARD: &str = "PRAXHUB";

/// Provider von THEORG selbst.
pub const PROVIDER_THEORG: &str = "THEORG";
