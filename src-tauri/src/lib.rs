//! Tauri-App-Schicht: UI, Konfiguration, Lebenszyklus. Die eigentliche Logik
//! liegt im Tauri-freien `connector_core`.

pub mod commands;
pub mod elevate;
pub mod state;
pub mod tray;

use state::AppState;
use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .manage(AppState::default())
        .setup(|app| {
            let handle = app.handle().clone();

            // Bei Login automatisch starten, damit der KIM-Watcher mitläuft.
            #[cfg(desktop)]
            {
                use tauri_plugin_autostart::ManagerExt;
                let _ = app.autolaunch().enable();
            }

            // System-Tray (Hintergrundbetrieb).
            tray::build(app.handle())?;

            // Fenster-Schließen = in den Tray minimieren statt beenden — der
            // KIM-Watcher läuft weiter. Beenden nur über das Tray-Menü.
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win.hide();
                    }
                });
                // Per Login-Autostart unsichtbar starten (nur Tray).
                if std::env::args().any(|a| a == "--autostart") {
                    let _ = window.hide();
                }
            }

            // Watcher starten, sobald die App läuft (innerhalb der Tokio-Runtime).
            tauri::async_runtime::spawn(async move {
                commands::start_watcher(&handle).await;
            });

            // Auto-Update: beim Start einmal den (signierten) Update-Feed prüfen und
            // ein verfügbares Update still im Hintergrund installieren. Nur im
            // Release-Build — im Dev gibt es keinen Updater-Kontext. Schlägt der Feed
            // fehl oder ist leer (Backend ohne Manifest), passiert nichts (nur Log),
            // damit der Start nie blockiert.
            #[cfg(not(debug_assertions))]
            {
                let updater_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    apply_pending_update(updater_handle).await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::get_status,
            commands::test_cloud_connection,
            commands::test_kim_connection,
            commands::test_z1db_connection,
            commands::bootstrap_z1_readonly,
            commands::register_with_pvs,
            commands::unregister_from_pvs,
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Start der Tauri-Anwendung");
}

/// Prüft den Updater-Feed und installiert ein verfügbares, signiertes Update still
/// im Hintergrund; danach Neustart, um es anzuwenden. Schlägt der Feed fehl oder ist
/// er leer (Backend noch ohne Manifest), wird nur geloggt — der Connector läuft
/// normal weiter. So ziehen künftige Releases automatisch durch, sobald getaggt.
#[cfg(not(debug_assertions))]
async fn apply_pending_update(handle: tauri::AppHandle) {
    use tauri_plugin_updater::UpdaterExt;
    let updater = match handle.updater() {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "Updater nicht verfügbar");
            return;
        }
    };
    match updater.check().await {
        Ok(Some(update)) => {
            tracing::info!(version = %update.version, "Update gefunden — installiere im Hintergrund");
            if let Err(e) = update
                .download_and_install(|_downloaded: usize, _total: Option<u64>| {}, || {})
                .await
            {
                tracing::warn!(error = %e, "Update-Installation fehlgeschlagen");
                return;
            }
            tracing::info!("Update installiert — Neustart, um es anzuwenden");
            handle.restart();
        }
        Ok(None) => tracing::debug!("Connector ist aktuell — kein Update"),
        Err(e) => tracing::warn!(error = %e, "Update-Check fehlgeschlagen (Feed erreichbar?)"),
    }
}
