//! Elevierte VDDS-INI-Operationen.
//!
//! Per-User-Install (asInvoker) wie nelly: die App läuft ohne Admin. Nur das
//! Schreiben der maschinenweiten `VDDS_MMI.INI` braucht einmalig Elevation —
//! darum relauncht [`crate::commands::register_with_pvs`] die eigene .exe per UAC
//! mit `--register-vdds`, und dieser Code hier macht die eigentliche Arbeit.

use connector_core::vdds::ini::{self, VddsRegistration};

/// Schreibt den VDDS-Eintrag. Rückgabe = Prozess-Exitcode (0 = ok).
pub fn run_register() -> i32 {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return 2,
    };
    let install_dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let reg = VddsRegistration { program_path: exe, install_dir };
    match ini::register(&ini::default_ini_path(), &reg) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

pub fn run_unregister() -> i32 {
    match ini::unregister(&ini::default_ini_path()) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
