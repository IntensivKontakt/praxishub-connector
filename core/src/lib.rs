//! Praxishub-Connector — portabler Logik-Kern (ohne Tauri/UI).
//!
//! Bausteine (siehe Linear PRA-15):
//!   * [`vdds`]  — Registrierung als BVS-Modul + Dokumentenablage in die PVS-Akte
//!   * [`kim`]   — nicht-destruktiver KIM/EBZ-Watcher (HKP-Genehmigungen erkennen)
//!   * [`z1db`]  — Z1-SQL-DB: Lesen (Status/HKP/Stammdaten) + strukturiertes
//!                 Rückschreiben (Kontakt/Adresse/CAVE/Anamnese) — `docs/Z1-DATABASE.md`
//!   * [`cloud`] — HTTPS-Anbindung an die Praxishub-Cloud
//!
//! Die App-Schicht (`src-tauri`) hält Konfiguration, UI und Lebenszyklus; dieser
//! Kern bleibt rein und unit-testbar.

pub mod cloud;
pub mod config;
pub mod crypto;
pub mod documents;
pub mod error;
pub mod kim;
pub mod matching;
pub mod paths;
pub mod patient_lookup;
pub mod status;
pub mod vdds;
pub mod z1db;

pub use config::ConnectorConfig;
pub use error::{ConnectorError, Result};
pub use status::{Health, SharedStatus, StatusSnapshot};
