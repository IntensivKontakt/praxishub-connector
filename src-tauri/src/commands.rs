//! Tauri-Commands (Brücke UI ↔ Core) + Watcher-Lebenszyklus.

use crate::state::AppState;
use connector_core::cloud::CloudClient;
use connector_core::config::ConnectorConfig;
use connector_core::kim::pop3::Pop3Client;
use connector_core::kim::Watcher;
use connector_core::status::{Component, Health, StatusSnapshot};
use connector_core::vdds::ini;
use tauri::{AppHandle, Manager, State};

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn get_config() -> Result<ConnectorConfig, String> {
    ConnectorConfig::load().map_err(err)
}

#[tauri::command]
pub async fn save_config(app: AppHandle, config: ConnectorConfig) -> Result<(), String> {
    config.save().map_err(err)?;
    restart_watcher(&app).await;
    Ok(())
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Result<StatusSnapshot, String> {
    // VDDS-Registrierung bei jedem Poll live prüfen.
    let vdds = match ini::is_registered(&ini::default_ini_path()) {
        Ok(true) => Component::new(Health::Ok, "registriert"),
        Ok(false) => Component::new(Health::Warn, "nicht registriert"),
        Err(e) => Component::new(Health::Err, format!("{e}")),
    };
    state.status.set_vdds(vdds);
    Ok(state.status.snapshot())
}

#[tauri::command]
pub async fn test_cloud_connection() -> Result<String, String> {
    let cfg = ConnectorConfig::load().map_err(err)?;
    if !cfg.cloud_ready() {
        return Err("Tenant, API-Key oder URL fehlt".into());
    }
    let client = CloudClient::new(&cfg).map_err(err)?;
    client.ping().await.map_err(err)
}

#[tauri::command]
pub async fn test_kim_connection() -> Result<String, String> {
    let cfg = ConnectorConfig::load().map_err(err)?;
    if !cfg.kim_ready() {
        return Err("KIM-Zugangsdaten unvollständig".into());
    }
    let mut c = Pop3Client::connect(&cfg.kim_host, cfg.kim_port, cfg.kim_allow_invalid_cert)
        .await
        .map_err(err)?;
    c.login(&cfg.kim_user, &cfg.kim_password).await.map_err(err)?;
    let (n, _) = c.stat().await.map_err(err)?;
    let _ = c.quit().await;
    Ok(format!("OK · {n} Nachricht(en) im Postfach"))
}

#[tauri::command]
pub fn register_with_pvs() -> Result<String, String> {
    spawn_elevated("--register-vdds")
}

#[tauri::command]
pub fn unregister_from_pvs() -> Result<String, String> {
    spawn_elevated("--unregister-vdds")
}

// ── intern ───────────────────────────────────────────────────────────────────

/// Relauncht die eigene .exe mit `arg` und löst den Windows-UAC-Prompt aus.
fn spawn_elevated(arg: &str) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(err)?;
    #[cfg(windows)]
    {
        let ps = format!(
            "Start-Process -FilePath '{}' -ArgumentList '{}' -Verb RunAs -Wait",
            exe.display(),
            arg
        );
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps])
            .spawn()
            .map_err(err)?;
        Ok("Windows-Berechtigungsabfrage gestartet …".into())
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, arg);
        Err("VDDS-Registrierung ist nur unter Windows verfügbar".into())
    }
}

pub(crate) async fn start_watcher(app: &AppHandle) {
    let state = app.state::<AppState>();
    let cfg = match ConnectorConfig::load() {
        Ok(c) => c,
        Err(_) => return,
    };

    // Dokumenten-Push (Variante B): hängt NUR an der Cloud, nicht am KIM-Postfach.
    // Läuft daher auch dann, wenn KIM gerade nicht erreichbar/eingerichtet ist.
    if cfg.cloud_ready() {
        let handle = connector_core::documents::spawn(cfg.clone());
        *state.doc_watcher.lock().await = Some(handle);
    }

    // KIM-Watcher: braucht zusätzlich ein konfiguriertes KIM-Postfach.
    if !cfg.kim_ready() || !cfg.cloud_ready() {
        state
            .status
            .set_kim(Component::new(Health::Warn, "wartet auf Konfiguration"));
        return;
    }
    match Watcher::spawn(cfg, state.status.clone()) {
        Ok(handle) => *state.watcher.lock().await = Some(handle),
        Err(e) => state.status.set_kim(Component::new(Health::Err, err(e))),
    }
}

pub(crate) async fn stop_watcher(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Some(handle) = state.watcher.lock().await.take() {
        handle.stop().await;
    }
    if let Some(handle) = state.doc_watcher.lock().await.take() {
        handle.stop().await;
    }
}

pub(crate) async fn restart_watcher(app: &AppHandle) {
    stop_watcher(app).await;
    start_watcher(app).await;
}
