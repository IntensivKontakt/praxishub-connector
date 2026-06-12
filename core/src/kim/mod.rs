//! KIM/EBZ-Watcher.
//!
//! Liest **nicht-destruktiv** (kein `DELE`, „leave on server", UIDL-Dedup) am
//! lokalen KIM-Clientmodul mit und erkennt genehmigte HKPs anhand des Headers
//! `X-KIM-Dienstkennung: EBZ;ANW;…`. Erkannte Nachrichten gehen roh an die Cloud.
//!
//! **Goldene Regel:** den EBZ-Workflow des PVS NIE stören — eine verlorene
//! Genehmigung ist ein Abrechnungsproblem. Darum: niemals löschen, immer Dedup.

pub mod ebz;
pub mod pop3;
pub mod watcher;

pub use watcher::{Watcher, WatcherHandle};
