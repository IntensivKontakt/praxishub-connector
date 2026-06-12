// Im Release-Build keine Konsole zeigen (Windows).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Elevierte Unterbefehle: werden vom UAC-Relaunch der App aufgerufen, um die
    // (maschinenweite) VDDS_MMI.INI zu schreiben — der Rest läuft per-user.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--register-vdds") {
        std::process::exit(praxishub_connector_lib::elevate::run_register());
    }
    if args.iter().any(|a| a == "--unregister-vdds") {
        std::process::exit(praxishub_connector_lib::elevate::run_unregister());
    }

    praxishub_connector_lib::run();
}
