//! Laufzeit-Status, den die UI pollt. Vom Watcher fortlaufend aktualisiert.

use serde::Serialize;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Ok,
    Warn,
    Err,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct Component {
    pub state: Health,
    pub detail: String,
}

impl Component {
    pub fn new(state: Health, detail: impl Into<String>) -> Self {
        Self { state, detail: detail.into() }
    }
    pub fn unknown() -> Self {
        Self { state: Health::Unknown, detail: String::new() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusSnapshot {
    pub version: String,
    pub vdds: Component,
    pub kim: Component,
    pub cloud: Component,
    pub last_hkp_at: Option<String>,
}

impl Default for StatusSnapshot {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            vdds: Component::unknown(),
            kim: Component::unknown(),
            cloud: Component::unknown(),
            last_hkp_at: None,
        }
    }
}

/// Threadsicher geteilter Status (Watcher schreibt, UI liest).
#[derive(Clone, Default)]
pub struct SharedStatus(Arc<Mutex<StatusSnapshot>>);

impl SharedStatus {
    pub fn snapshot(&self) -> StatusSnapshot {
        self.0.lock().unwrap().clone()
    }

    pub fn set_kim(&self, c: Component) {
        self.0.lock().unwrap().kim = c;
    }

    pub fn set_cloud(&self, c: Component) {
        self.0.lock().unwrap().cloud = c;
    }

    pub fn set_vdds(&self, c: Component) {
        self.0.lock().unwrap().vdds = c;
    }

    pub fn mark_hkp_now(&self) {
        self.0.lock().unwrap().last_hkp_at = Some(chrono::Local::now().format("%Y-%m-%d %H:%M").to_string());
    }
}
