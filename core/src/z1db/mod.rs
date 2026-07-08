//! Z1-SQL-Datenbank: Lesen (Status/HKP/Stammdaten) und **strukturiertes
//! Rückschreiben** (Kontaktdaten, Adresse, CAVE/Risikoanamnese, Krankenanamnese).
//!
//! Vollständige Schema-/Verfahrensreferenz: `docs/Z1-DATABASE.md`. Die hier
//! umgesetzten Schreibpfade wurden am Live-Z1 verifiziert (siehe ebd. Abschnitt 7):
//!   * Kontakt/Adresse → `UPDATE ADR` (bestehende Zeile)
//!   * CAVE/Allergien  → additiv an `PAT.ANAMNESE` (Risikoanamnese, `varchar(80)`)
//!   * Krankenanamnese → `INSERT INTO PATINFO` (ART=1) — exakt wie Nelly
//!
//! Eigenständige Hintergrund-Schleifen (analog `documents::spawn`):
//!   * [`hkp::spawn`]       — liest neue HKP-Entscheidungen aus `EBZ` + Voll-HKP aus
//!                            `FILEPOOL` und meldet sie der Cloud (ersetzt KIM).
//!   * [`writeback::spawn`] — holt Aufnahme-Bündel aus der Cloud und schreibt sie
//!                            (je nach Toggle) strukturiert in Z1 zurück.
//!   * [`control::spawn`]   — nächtlicher Aggregat-Sync (Praxis-Steuerung):
//!                            Honorar/Zahlungen/Forderungen/offene Leistungen.
//!
//! **Goldene Regel (analog KIM-Watcher):** den PVS-Betrieb nie stören — nur
//! additiv/gezielt schreiben, Datensatz vorher prüfen, Transaktion +
//! Zeilenzahl-Assertion, `RINFO` app-treu setzen. Jede Fähigkeit ist per Toggle
//! einzeln aktivierbar.

pub mod bootstrap;
pub mod client;
pub mod control;
pub mod hkp;
pub mod lookup;
pub mod patient_match;
pub mod writeback;

pub use bootstrap::create_readonly_login;
pub use client::{connect, Z1Connection};
pub use control::spawn as spawn_control_sync;
pub use hkp::spawn as spawn_hkp_poller;
pub use lookup::resolve_patient;
pub use patient_match::spawn as spawn_patient_match;
pub use writeback::{
    apply_writeback, spawn as spawn_writeback, ContactData, PatientWriteback, WritebackReport,
};

/// Handle einer eigenständigen Z1-DB-Hintergrundschleife (HKP-Poller /
/// Writeback-Loop). `stop()` signalisiert und wartet auf sauberes Ende.
pub struct LoopHandle {
    shutdown: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<()>,
}

impl LoopHandle {
    pub(crate) fn new(
        shutdown: tokio::sync::watch::Sender<bool>,
        join: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self { shutdown, join }
    }

    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = self.join.await;
    }
}
