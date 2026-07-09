//! Watcher-Loop: pollt das KIM-Postfach, filtert EBZ-Genehmigungen, meldet sie
//! an die Cloud — strikt nicht-destruktiv und idempotent (UIDL-Dedup).

use crate::cloud::{CloudClient, HkpReport};
use crate::config::ConnectorConfig;
use crate::error::Result;
use crate::kim::ebz;
use crate::kim::pop3::Pop3Client;
use crate::paths;
use crate::status::{Component, Health, SharedStatus};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Persistenter UIDL-Dedup-Store (überlebt Neustarts).
struct SeenStore {
    set: HashSet<String>,
}

impl SeenStore {
    fn load() -> Result<Self> {
        let path = paths::seen_store_file()?;
        let set = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => HashSet::new(),
        };
        Ok(Self { set })
    }

    fn contains(&self, uid: &str) -> bool {
        self.set.contains(uid)
    }

    fn insert(&mut self, uid: String) {
        if self.set.insert(uid) {
            let _ = self.persist();
        }
    }

    /// Hält den Store auf den aktuell am Server vorhandenen UIDLs (verhindert
    /// unbegrenztes Wachstum, sobald das PVS alte Mails wegräumt).
    fn retain_only(&mut self, current: &HashSet<String>) {
        let before = self.set.len();
        self.set.retain(|u| current.contains(u));
        if self.set.len() != before {
            let _ = self.persist();
        }
    }

    fn persist(&self) -> Result<()> {
        let path = paths::seen_store_file()?;
        std::fs::write(path, serde_json::to_vec(&self.set)?)?;
        Ok(())
    }
}

pub struct Watcher {
    cfg: ConnectorConfig,
    cloud: CloudClient,
    status: SharedStatus,
    seen: SeenStore,
}

pub struct WatcherHandle {
    shutdown: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<()>,
}

impl WatcherHandle {
    /// Signalisiert Stopp und wartet auf das saubere Ende des Loops.
    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.join.await;
    }
}

impl Watcher {
    /// Startet den Watcher als Hintergrund-Task auf der aktuellen Tokio-Runtime.
    pub fn spawn(cfg: ConnectorConfig, status: SharedStatus) -> Result<WatcherHandle> {
        let cloud = CloudClient::new(&cfg)?;
        let seen = SeenStore::load()?;
        let watcher = Watcher { cfg, cloud, status, seen };
        let (tx, rx) = tokio::sync::watch::channel(false);
        let join = tokio::spawn(watcher.run(rx));
        Ok(WatcherHandle { shutdown: tx, join })
    }

    async fn run(mut self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let period = Duration::from_secs(self.cfg.kim_poll_seconds.max(10));
        let mut ticker = tokio::time::interval(period);
        info!(host = %self.cfg.kim_host, port = self.cfg.kim_port, "KIM-Watcher gestartet");
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let last_error = match self.poll_once().await {
                        Ok(n) => {
                            self.status.set_kim(Component::new(
                                Health::Ok,
                                format!("aktiv · {n} HKP(s) zuletzt gemeldet"),
                            ));
                            None
                        }
                        Err(e) => {
                            warn!(error = %e, "KIM-Poll fehlgeschlagen");
                            let s = e.to_string();
                            self.status.set_kim(Component::new(Health::Err, format!("Fehler: {s}")));
                            Some(s)
                        }
                    };
                    // Hinweis: Der Dokumenten-Push (Variante B) läuft als eigene,
                    // KIM-unabhängige Schleife (`crate::documents::spawn`) — er darf
                    // NICHT am KIM-Zyklus hängen, weil KIM oft nicht erreichbar ist,
                    // die Z1-/Cloud-Seite aber sehr wohl.
                    // Heartbeat IMMER senden (auch bei Fehler) → Backend-Watchdog
                    // sieht sofort Stille (Dienst tot) oder gemeldete Fehler.
                    let vdds_ok = self.status.snapshot().vdds.state == Health::Ok;
                    // Legacy-KIM-Watcher (nicht mehr gestartet) – Heartbeat läuft jetzt eigenständig.
                    match self.cloud.heartbeat(vdds_ok, true, false, last_error.as_deref()).await {
                        Ok(()) => self.status.set_cloud(Component::new(Health::Ok, "verbunden")),
                        Err(e) => self.status.set_cloud(Component::new(Health::Warn, format!("Cloud: {e}"))),
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("KIM-Watcher gestoppt");
                        self.status.set_kim(Component::new(Health::Warn, "gestoppt"));
                        break;
                    }
                }
            }
        }
    }

    /// Ein Poll-Zyklus. Gibt die Anzahl in diesem Zyklus gemeldeter HKPs zurück.
    async fn poll_once(&mut self) -> Result<usize> {
        let mut client = Pop3Client::connect(
            &self.cfg.kim_host,
            self.cfg.kim_port,
            self.cfg.kim_allow_invalid_cert,
        )
        .await?;
        client.login(&self.cfg.kim_user, &self.cfg.kim_password).await?;

        let entries = client.uidl_all().await?;
        let current: HashSet<String> = entries.iter().map(|(_, u)| u.clone()).collect();

        let mut reported = 0usize;
        for (msg_no, uid) in &entries {
            if self.seen.contains(uid) {
                continue;
            }
            // Günstig nur die Header ziehen, um die Dienstkennung zu prüfen.
            let headers = client.top(*msg_no, 0).await?;
            if !ebz::is_ebz_approval(&headers) {
                // Nicht-EBZ: als gesehen markieren, aber NIE löschen.
                self.seen.insert(uid.clone());
                continue;
            }

            debug!(%uid, "EBZ-Genehmigung erkannt — Volltext ziehen");
            let raw = client.retr(*msg_no).await?;
            let summary = ebz::summarize(&raw);
            let report = HkpReport {
                source_uidl: uid.clone(),
                dienstkennung: summary.dienstkennung,
                message_id: summary.message_id,
                received_at: summary.received_at,
                raw_message_b64: STANDARD.encode(raw.as_bytes()),
            };

            match self.cloud.report_hkp(&report).await {
                Ok(()) => {
                    info!(%uid, "HKP an Cloud gemeldet");
                    self.seen.insert(uid.clone()); // erst NACH Erfolg → sonst Retry
                    self.status.mark_hkp_now();
                    reported += 1;
                }
                Err(e) => {
                    // Nicht als gesehen markieren → nächster Zyklus versucht es erneut.
                    warn!(%uid, error = %e, "HKP-Meldung fehlgeschlagen, Retry im nächsten Zyklus");
                }
            }
        }

        // Niemals DELE — Postfach bleibt für das PVS unangetastet.
        let _ = client.quit().await;

        // Dedup-Store auf Server-Bestand eindampfen.
        self.seen.retain_only(&current);
        // Heartbeat wird zentral im run-Loop gesendet (immer, auch bei Fehler).

        Ok(reported)
    }
}
