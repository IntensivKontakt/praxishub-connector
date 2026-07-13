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

    // Potenzialanalyse (Sales/vor Ort): liest read-only die PVS-DB, bewertet die
    // ökonomischen Hebel und schreibt einen lokalen HTML-Report — OHNE Cloud.
    if args.iter().any(|a| a == "--potenzialanalyse") {
        std::process::exit(run_potential_analysis());
    }

    // Vom PVS via VDDS-media aufgerufen? (Argument = Pfad auf eine .ini-Datei)
    if let Some(ini) = args.iter().skip(1).find(|a| connector_core::vdds::media::is_media_invocation(a)) {
        let path = std::path::Path::new(ini);

        // MMOEXPORT-Abruf (Pull): ConVis holt eine per MMOINFIMPORT angekündigte
        // Dokumentkopie ab — wir tragen den Dateipfad in die INI ein und quittieren.
        // Muss VOR der Patienten-Behandlung stehen (Export-INI hat keinen Patienten).
        if connector_core::vdds::media::is_export_request(path) {
            let exchange = connector_core::ConnectorConfig::load()
                .map(|c| c.exchange_dir_path())
                .unwrap_or_else(|_| std::env::temp_dir());
            let code = match connector_core::vdds::media::handle_export_request(path, &exchange) {
                Ok(out) if out.missing.is_empty() && out.resolved > 0 => 0,
                _ => 1,
            };
            std::process::exit(code);
        }

        let code = match connector_core::vdds::media::handle_invocation(path) {
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

/// Potenzialanalyse als Einmal-Lauf (Sales-Einsatz, vor dem Kauf): braucht nur
/// die im Connector hinterlegten PVS-Lesezugangsdaten — KEINE Cloud, kein
/// Tenant. Erhebt die Kennzahlen read-only, bewertet sie
/// ([`connector_core::analysis::evaluate`]) und schreibt HTML + JSON nach
/// `…\logs\potenzialanalyse-<datum>.html/.json`; das HTML wird direkt geöffnet.
///
/// Aufruf: `praxishub-connector.exe --potenzialanalyse`
fn run_potential_analysis() -> i32 {
    let report = |msg: &str| {
        println!("{msg}");
        eprintln!("{msg}");
        if let Ok(dir) = connector_core::paths::log_dir() {
            let _ = std::fs::write(dir.join("potenzialanalyse-fehler.txt"), msg);
        }
    };
    let cfg = match connector_core::ConnectorConfig::load() {
        Ok(c) => c,
        Err(e) => {
            report(&format!("FEHLER: Konfiguration nicht lesbar: {e}"));
            return 2;
        }
    };
    if !cfg.z1db_read_ready() {
        report(
            "FEHLER: Kein PVS-DB-Lesezugriff konfiguriert. Im Connector unter \
             „Z1-Datenbank" Server + Read-only-Login eintragen (Cloud ist NICHT nötig).",
        );
        return 2;
    }
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            report(&format!("FEHLER: Runtime: {e}"));
            return 2;
        }
    };
    let result: Result<(std::path::PathBuf, String), String> = rt.block_on(async {
        let mut conn = connector_core::z1db::connect(
            &cfg.z1_db_server,
            &cfg.z1_db_database,
            &cfg.z1_db_user,
            &cfg.z1_db_password,
            cfg.z1_db_trust_cert,
        )
        .await
        .map_err(|e| format!("PVS-DB nicht erreichbar: {e}"))?;
        let today = chrono::Local::now().date_naive();
        let inputs = connector_core::z1db::analysis::collect_inputs(&mut conn, today).await;
        let rep = connector_core::analysis::evaluate(&inputs);
        let dir = connector_core::paths::log_dir().map_err(|e| e.to_string())?;
        let stem = format!("potenzialanalyse-{}", today.format("%Y%m%d"));
        let html_path = dir.join(format!("{stem}.html"));
        std::fs::write(&html_path, connector_core::analysis::render_html(&rep))
            .map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(&rep).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{stem}.json")), json).map_err(|e| e.to_string())?;
        Ok((html_path, format!("{} Befunde", rep.findings.len())))
    });
    match result {
        Ok((html_path, summary)) => {
            report(&format!("OK: {summary} — Report: {}", html_path.display()));
            // Report direkt im Standardbrowser öffnen (Sales-Situation).
            #[cfg(windows)]
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", &html_path.to_string_lossy()])
                .spawn();
            0
        }
        Err(e) => {
            report(&format!("FEHLER: {e}"));
            1
        }
    }
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

    // Stabile, dateinamen-sichere MMOID aus dem PDF-Namen — unter ihr wird die
    // Kopie gestaged, damit ein MMOEXPORT-Pull von ConVis sie findet.
    let stem: String = pdf_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let mmoid = format!("PHTEST_{stem}");

    match media::import_once_diagnostic(&import_program, &req, &cfg.exchange_dir_path(), &mmoid) {
        Ok(d) => {
            let ready_done = d.ready.as_deref() == Some("1");
            let ok = ready_done && d.errorlevel.as_deref() == Some("0");
            let verdict = if ok {
                "OK · READY=1 & ERRORLEVEL=0 — Import erfolgreich. In Z1 beim Patienten im Archiv prüfen."
            } else if ready_done {
                "ABGELEHNT · READY=1, aber ERRORLEVEL != 0 — Z1 hat den Import nicht übernommen (Code/Text unten)."
            } else {
                "UNKLAR · kein READY=1 (Handshake) — MmoInfIm evtl. synchron oder INI abgelehnt; Exit-Code + INI unten ansehen."
            };
            report(&format!(
                "Praxishub --push-test · Diagnose\n\
                 ================================\n\
                 {verdict}\n\n\
                 MMOINFIMPORT : {prog}\n\
                 Exit-Code    : {code:?} (success={succ})\n\
                 READY        : {ready}\n\
                 ERRORLEVEL   : {el}\n\
                 ERRORTEXT    : {etext}\n\n\
                 Dateien im Austauschordner nach dem Aufruf:\n{files}\n\n\
                 ===== Gesendete VDDS_MMO.INI =====\n{sent}\n\
                 ===== VDDS_MMO.INI NACH dem Aufruf (Antwort von MmoInfIm) =====\n{after}",
                prog = import_program.display(),
                code = d.exit_code,
                succ = d.exit_success,
                ready = d.ready.as_deref().unwrap_or("(keins)"),
                el = d.errorlevel.as_deref().unwrap_or("(keins)"),
                etext = d.errortext.as_deref().unwrap_or("(keiner)"),
                files = if d.exchange_files.is_empty() {
                    "(keine)".to_string()
                } else {
                    d.exchange_files.join("\n")
                },
                sent = d.sent_ini,
                after = d.ini_after,
            ));
            if ok {
                0
            } else {
                3
            }
        }
        Err(e) => {
            report(&format!("FEHLER beim Aufruf von MMOINFIMPORT: {e}"));
            2
        }
    }
}
