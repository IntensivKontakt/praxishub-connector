//! Praxishub-Connector — portabler Logik-Kern (ohne Tauri/UI).
//!
//! Drei Bausteine (siehe Linear PRA-15):
//!   * [`vdds`]  — Registrierung als BVS-Modul + Dokumentenablage in die PVS-Akte
//!   * [`kim`]   — nicht-destruktiver KIM/EBZ-Watcher (HKP-Genehmigungen erkennen)
//!   * [`cloud`] — HTTPS-Anbindung an die Praxishub-Cloud
//!
//! Die App-Schicht (`src-tauri`) hält Konfiguration, UI und Lebenszyklus; dieser
//! Kern bleibt rein und unit-testbar.

pub mod cloud;
pub mod config;
pub mod crypto;
pub mod error;
pub mod kim;
pub mod paths;
pub mod status;
pub mod vdds;

pub use config::ConnectorConfig;
pub use error::{ConnectorError, Result};
pub use status::{Health, SharedStatus, StatusSnapshot};
