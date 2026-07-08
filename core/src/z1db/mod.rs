//! Z1-SQL-Datenbank: Lesen (Status/HKP/Stammdaten) und **strukturiertes
//! Rückschreiben** (Kontaktdaten, Adresse, CAVE/Risikoanamnese, Krankenanamnese).
//!
//! Vollständige Schema-/Verfahrensreferenz: `docs/Z1-DATABASE.md`. Die hier
//! umgesetzten Schreibpfade wurden am Live-Z1 verifiziert (siehe ebd. Abschnitt 7):
//!   * Kontakt/Adresse → `UPDATE ADR` (bestehende Zeile)
//!   * CAVE/Allergien  → additiv an `PAT.ANAMNESE` (Risikoanamnese, `varchar(80)`)
//!   * Krankenanamnese → `INSERT INTO PATINFO` (ART=1) — exakt wie Nelly
//!
//! **Goldene Regel (analog KIM-Watcher):** den PVS-Betrieb nie stören. Deshalb:
//! nur additiv/gezielt schreiben, jeden Datensatz vor dem Schreiben prüfen
//! (Adress-Freigabe), Transaktion + Zeilenzahl-Assertion, `RINFO` app-treu setzen.
//! Jede Fähigkeit ist über einen eigenen Config-Toggle einzeln aktivierbar.

pub mod bootstrap;
pub mod client;
pub mod writeback;

pub use bootstrap::create_readonly_login;
pub use client::{connect, Z1Connection};
pub use writeback::{apply_writeback, ContactData, PatientWriteback, WritebackReport};
