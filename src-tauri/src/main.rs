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

    // Vom PVS via VDDS-media aufgerufen? (Argument = Pfad auf eine .ini-Datei)
    if let Some(ini) = args.iter().skip(1).find(|a| connector_core::vdds::media::is_media_invocation(a)) {
        let code = match connector_core::vdds::media::handle_invocation(std::path::Path::new(ini)) {
            Ok(_) => 0,
            Err(_) => 1,
        };
        std::process::exit(code);
    }

    praxishub_connector_lib::run();
}
