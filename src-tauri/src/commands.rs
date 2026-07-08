//! Tauri-Commands (Brücke UI ↔ Core) + Watcher-Lebenszyklus.

use crate::state::AppState;
use connector_core::cloud::CloudClient;
use connector_core::config::ConnectorConfig;
use connector_core::kim::pop3::Pop3Client;
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

/// Verbindungstest gegen die Z1-DB mit dem gespeicherten Read-only-Login.
#[tauri::command]
pub async fn test_z1db_connection() -> Result<String, String> {
    let cfg = ConnectorConfig::load().map_err(err)?;
    if !cfg.z1db_read_ready() {
        return Err("Z1-DB-Zugang unvollständig (Server/DB/Benutzer/Passwort)".into());
    }
    let mut conn = connector_core::z1db::connect(
        &cfg.z1_db_server,
        &cfg.z1_db_database,
        &cfg.z1_db_user,
        &cfg.z1_db_password,
        cfg.z1_db_trust_cert,
    )
    .await
    .map_err(err)?;
    let version = conn.ping().await.map_err(err)?;
    let first = version.lines().next().unwrap_or("").trim();
    Ok(format!("verbunden · {first}"))
}

/// Legt aus **temporär** eingegebenen Admin-Zugangsdaten den dedizierten
/// Read-only-Login `praxishub_ro` an und speichert NUR diesen (DPAPI) in der
/// Config. Die Admin-Zugangsdaten werden nicht persistiert — die UI weist den
/// Nutzer an, sie danach zu verwerfen.
#[tauri::command]
pub async fn bootstrap_z1_readonly(
    server: String,
    admin_user: String,
    admin_password: String,
    ro_password: String,
    trust_cert: bool,
) -> Result<String, String> {
    let database = "Z1";
    let ro_user = "praxishub_ro";
    connector_core::z1db::create_readonly_login(
        &server,
        database,
        &admin_user,
        &admin_password,
        ro_user,
        &ro_password,
        trust_cert,
    )
    .await
    .map_err(err)?;

    let mut cfg = ConnectorConfig::load().map_err(err)?;
    cfg.z1_db_server = server;
    cfg.z1_db_database = database.to_string();
    cfg.z1_db_user = ro_user.to_string();
    cfg.z1_db_password = ro_password;
    cfg.z1_db_trust_cert = trust_cert;
    cfg.save().map_err(err)?;

    Ok("Read-only-Login angelegt und gespeichert. Die Admin-Zugangsdaten wurden \
        NICHT gespeichert — du kannst sie jetzt wieder löschen."
        .to_string())
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

    // Z1-HKP-Poller (EBZ-Status → Cloud): braucht Z1-DB-Lesen + Cloud. Ersetzt
    // perspektivisch den KIM-Watcher, läuft aber unabhängig davon.
    if cfg.z1db_read_ready() && cfg.cloud_ready() {
        let handle = connector_core::z1db::spawn_hkp_poller(cfg.clone());
        *state.hkp_poller.lock().await = Some(handle);
    }

    // Z1-Writeback (Cloud → Z1): braucht schreibfähigen Login + aktiven Toggle.
    if cfg.z1db_write_ready() && cfg.cloud_ready() {
        let handle = connector_core::z1db::spawn_writeback(cfg.clone());
        *state.writeback_loop.lock().await = Some(handle);
    }

    // Eigenständiger Heartbeat (KIM-unabhängig) – hält den Connector in der Cloud „lebendig",
    // meldet kim_watching=false + hkp_db_watching. Ersetzt den Heartbeat des alten KIM-Watchers.
    if cfg.cloud_ready() {
        let handle = connector_core::heartbeat::spawn(cfg.clone(), state.status.clone());
        *state.heartbeat_loop.lock().await = Some(handle);
    }

    // Z1-PATID-Nachmatch: Alt-Patienten ohne PVS-Nummer über die Z1-DB auflösen.
    if cfg.z1db_read_ready() && cfg.cloud_ready() {
        let handle = connector_core::z1db::spawn_patient_match(cfg.clone());
        *state.patient_match_loop.lock().await = Some(handle);
    }

    // KIM/EBZ-Weg ist abgelöst (HKP-Fälle kommen jetzt direkt aus der Z1-DB) – kein KIM-Watcher mehr.
    state
        .status
        .set_kim(Component::new(Health::Unknown, "abgelöst (Z1-DB)"));
}

pub(crate) async fn stop_watcher(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Erst aus den Locks nehmen (Guard-Temporäre sofort fallen lassen), dann
    // außerhalb des Lock-Scopes awaiten — sonst lebt der MutexGuard über das
    // .await hinaus und borgt `state` zu lange (E0597).
    let watcher = state.watcher.lock().await.take();
    let doc_watcher = state.doc_watcher.lock().await.take();
    let hkp_poller = state.hkp_poller.lock().await.take();
    let writeback_loop = state.writeback_loop.lock().await.take();
    let heartbeat_loop = state.heartbeat_loop.lock().await.take();
    let patient_match_loop = state.patient_match_loop.lock().await.take();
    if let Some(handle) = watcher {
        handle.stop().await;
    }
    if let Some(handle) = doc_watcher {
        handle.stop().await;
    }
    if let Some(handle) = hkp_poller {
        handle.stop().await;
    }
    if let Some(handle) = writeback_loop {
        handle.stop().await;
    }
    if let Some(handle) = heartbeat_loop {
        handle.stop().await;
    }
    if let Some(handle) = patient_match_loop {
        handle.stop().await;
    }
}

pub(crate) async fn restart_watcher(app: &AppHandle) {
    stop_watcher(app).await;
    start_watcher(app).await;
}
