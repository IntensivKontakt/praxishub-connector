//! Pfade im Per-User-AppData (Per-User-Install, kein Admin nötig).

use crate::error::{ConnectorError, Result};
use directories::ProjectDirs;
use std::path::PathBuf;

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("ai", "praxishub", "connector")
        .ok_or_else(|| ConnectorError::Config("Kein Benutzer-Konfigverzeichnis gefunden".into()))
}

/// Verzeichnis für Konfiguration & Zustands-Dateien (wird bei Bedarf erstellt).
pub fn config_dir() -> Result<PathBuf> {
    let dir = project_dirs()?.config_dir().to_path_buf();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `config.json`
pub fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.json"))
}

/// Persistierter UIDL-Dedup-Store des KIM-Watchers (`seen_uidls.json`).
pub fn seen_store_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("seen_uidls.json"))
}

/// Log-Verzeichnis.
pub fn log_dir() -> Result<PathBuf> {
    let dir = project_dirs()?.data_dir().join("logs");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
