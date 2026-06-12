//! Tauri-App-Schicht: UI, Konfiguration, Lebenszyklus. Die eigentliche Logik
//! liegt im Tauri-freien `connector_core`.

pub mod commands;
pub mod elevate;
pub mod state;

use state::AppState;
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

            // Watcher starten, sobald die App läuft (innerhalb der Tokio-Runtime).
            tauri::async_runtime::spawn(async move {
                commands::start_watcher(&handle).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::get_status,
            commands::test_cloud_connection,
            commands::test_kim_connection,
            commands::register_with_pvs,
            commands::unregister_from_pvs,
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Start der Tauri-Anwendung");
}
