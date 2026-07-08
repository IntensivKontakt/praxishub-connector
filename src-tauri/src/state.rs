use connector_core::documents::DocWatcherHandle;
use connector_core::kim::WatcherHandle;
use connector_core::status::SharedStatus;
use connector_core::z1db::LoopHandle;
use tokio::sync::Mutex;

/// Globaler App-Zustand (von Tauri verwaltet, in Commands via `State` erreichbar).
#[derive(Default)]
pub struct AppState {
    pub status: SharedStatus,
    /// Aktiver KIM-Watcher (None = gestoppt).
    pub watcher: Mutex<Option<WatcherHandle>>,
    /// Aktive Dokumenten-Push-Schleife (None = gestoppt). Läuft KIM-unabhängig.
    pub doc_watcher: Mutex<Option<DocWatcherHandle>>,
    /// Z1-HKP-Poller (EBZ-Status → Cloud). None = gestoppt.
    pub hkp_poller: Mutex<Option<LoopHandle>>,
    /// Z1-Writeback-Schleife (Cloud → Z1). None = gestoppt.
    pub writeback_loop: Mutex<Option<LoopHandle>>,
    /// Eigenständiger Heartbeat (KIM-unabhängig). None = gestoppt.
    pub heartbeat_loop: Mutex<Option<LoopHandle>>,
    /// Z1-PATID-Nachmatch-Schleife. None = gestoppt.
    pub patient_match_loop: Mutex<Option<LoopHandle>>,
}
