use connector_core::kim::WatcherHandle;
use connector_core::status::SharedStatus;
use tokio::sync::Mutex;

/// Globaler App-Zustand (von Tauri verwaltet, in Commands via `State` erreichbar).
#[derive(Default)]
pub struct AppState {
    pub status: SharedStatus,
    /// Aktiver KIM-Watcher (None = gestoppt).
    pub watcher: Mutex<Option<WatcherHandle>>,
}
