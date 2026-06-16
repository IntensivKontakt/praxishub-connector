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

    // Manueller Diagnose-Push: legt ein PDF direkt über Z1s MMOINFIMPORT ab —
    // umgeht die Cloud komplett. Zum Verifizieren am echten Z1, ob der
    // Dokumenten-Import überhaupt funktioniert, bevor das Backend steht.
    if let Some(pos) = args.iter().position(|a| a == "--push-test") {
        std::process::exit(run_push_test(&args[pos + 1..]));
    }

    // Vom PVS via VDDS-media aufgerufen? (Argument = Pfad auf eine .ini-Datei)
    if let Some(ini) = args.iter().skip(1).find(|a| connector_core::vdds::media::is_media_invocation(a)) {
        let code = match connector_core::vdds::media::handle_invocation(std::path::Path::new(ini)) {
            Ok(patient) => {
                // Variante A: Z1 hat einen Patienten geöffnet und übergibt uns die
                // echte PATID — jetzt dessen offene Praxishub-Dokumente ablegen.
                // Best effort, kurzlebig: Z1 darf dadurch nie blockieren/fehlschlagen.
                if let Ok(cfg) = connector_core::ConnectorConfig::load() {
                    if cfg.cloud_ready() {
                        if let Ok(rt) = tokio::runtime::Runtime::new() {
                            let _ = rt.block_on(
                                connector_core::documents::file_pending_for_patient(&cfg, &patient),
                            );
                        }
                    }
                }
                0
            }
            Err(_) => 1,
        };
        std::process::exit(code);
    }

    praxishub_connector_lib::run();
}

/// Manueller Test des VDDS-media-Dokumenten-Push gegen das echte Z1 — **ohne**
/// Cloud/Backend. Holt `MMOINFIMPORT` aus der `VDDS_MMI.INI` und schiebt das
/// angegebene PDF an den genannten (Test-)Patienten.
///
/// Aufruf (auf dem Praxis-PC, in cmd/PowerShell):
/// ```text
/// praxishub-connector.exe --push-test <pfad.pdf> \
///     [--patid <ID>] [--name <Nachname> --vorname <Vorname> --dob TT.MM.JJJJ] [--hkp]
/// ```
/// Da der Release-Build keine Konsole hat, wird das Ergebnis zusätzlich nach
/// `…\logs\push-test-result.txt` geschrieben. Exit: 0 = Z1 nahm das PDF an,
/// 3 = Z1 lehnte den Patienten ab, 2 = Konfigurations-/Aufruffehler.
fn run_push_test(args: &[String]) -> i32 {
    use connector_core::vdds::{ini, media};
    use std::path::PathBuf;

    let mut pdf: Option<String> = None;
    let (mut patid, mut name, mut vorname, mut dob) =
        (String::new(), String::new(), String::new(), String::new());
    let mut kind = media::DocumentKind::Anamnese;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--patid" => patid = it.next().cloned().unwrap_or_default(),
            "--name" => name = it.next().cloned().unwrap_or_default(),
            "--vorname" => vorname = it.next().cloned().unwrap_or_default(),
            "--dob" => dob = it.next().cloned().unwrap_or_default(),
            "--hkp" => kind = media::DocumentKind::Hkp,
            s if !s.starts_with("--") && pdf.is_none() => pdf = Some(s.to_string()),
            _ => {}
        }
    }

    // Ergebnis auf stdout/stderr UND in eine Datei (Release-Build hat keine Konsole).
    let report = |msg: &str| {
        println!("{msg}");
        eprintln!("{msg}");
        if let Ok(dir) = connector_core::paths::log_dir() {
            let _ = std::fs::write(dir.join("push-test-result.txt"), msg);
        }
    };

    let Some(pdf) = pdf else {
        report(
            "FEHLER: PDF-Pfad fehlt.\nAufruf: praxishub-connector.exe --push-test <pfad.pdf> \
             [--patid <ID> | --name <Nachname> --vorname <Vorname> --dob TT.MM.JJJJ] [--hkp]",
        );
        return 2;
    };
    let pdf_path = PathBuf::from(&pdf);

    let import_program = match ini::read_pvs_import_program(&ini::default_ini_path()) {
        Ok(Some(p)) => p,
        Ok(None) => {
            report(
                "FEHLER: Kein MMOINFIMPORT in der VDDS_MMI.INI — bietet dieses Z1 einen \
                 Info-Import an und ist Praxishub registriert?",
            );
            return 2;
        }
        Err(e) => {
            report(&format!("FEHLER beim Lesen der VDDS_MMI.INI: {e}"));
            return 2;
        }
    };

    let cfg = connector_core::ConnectorConfig::load().unwrap_or_default();
    let patient = media::PatientContext {
        patient_id: patid,
        last_name: name,
        first_name: vorname,
        birth_date: dob,
    };
    let req = media::ImportRequest { patient: &patient, pdf_path: &pdf_path, kind };

    match media::file_document(&import_program, &req, &cfg.exchange_dir_path()) {
        Ok(media::FilingOutcome::Filed { .. }) => {
            report(&format!(
                "OK: Z1 hat das PDF angenommen (MMOINFIMPORT={}). \
                 Jetzt in Z1 beim Patienten im Archiv prüfen, ob es sichtbar ist.",
                import_program.display()
            ));
            0
        }
        Ok(media::FilingOutcome::Deferred(reason)) => {
            report(&format!(
                "ABGELEHNT: MMOINFIMPORT lief, übernahm den Patienten aber nicht ({reason}). \
                 PATID bzw. Name+Geburtsdatum prüfen."
            ));
            3
        }
        Err(e) => {
            report(&format!("FEHLER beim Aufruf von MMOINFIMPORT: {e}"));
            2
        }
    }
}
