//! Eigenständiger Heartbeat-Loop (KIM-unabhängig).
//!
//! Früher hing der Heartbeat am KIM-Watcher; der KIM/EBZ-Weg ist abgelöst (HKP-Fälle
//! kommen jetzt direkt aus der Z1-DB). Der Heartbeat läuft daher als eigene Schleife,
//! damit die Cloud den Connector auch ohne KIM-Postfach als „lebendig" sieht. Meldet
//! `kim_watching=false` und `hkp_db_watching` = ob der Z1-DB-HKP-Sync läuft.

use std::time::Duration;

use tracing::warn;

use crate::cloud::CloudClient;
use crate::config::ConnectorConfig;
use crate::status::{Component, Health, SharedStatus};
use crate::z1db::LoopHandle;

pub fn spawn(cfg: ConnectorConfig, status: SharedStatus) -> LoopHandle {
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        let cloud = match CloudClient::new(&cfg) {
            Ok(c) => c,
            Err(e) => {
                warn!(error=%e, "Heartbeat: Cloud-Client fehlgeschlagen — Schleife beendet");
                return;
            }
        };
        let hkp_db = cfg.z1db_read_ready();
        // Gemeldete Dokument-Capabilities (config-abhängig, ändert sich im
        // Loop-Leben nicht) — einmal berechnen.
        let doc_kinds = cfg.supported_document_kinds();
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let vdds_ok = status.snapshot().vdds.state == Health::Ok;
                    // kim_watching=false (Weg abgelöst); last_error=None (KIM-Timeouts
                    // sollen nicht mehr als Fehler erscheinen).
                    match cloud.heartbeat(vdds_ok, false, hkp_db, &doc_kinds, None).await {
                        Ok(()) => status.set_cloud(Component::new(Health::Ok, "verbunden")),
                        Err(e) => status.set_cloud(Component::new(Health::Warn, format!("Cloud: {e}"))),
                    }
                }
                _ = rx.changed() => {
                    if *rx.borrow() { break; }
                }
            }
        }
    });
    LoopHandle::new(tx, join)
}
