//! THEDEX-Ordner: Selbsteintrag, Eingang, FEHLER-Ordner, Versand.

use chrono::TimeZone;
use chrono_tz::Europe::Berlin;
use praxishub_thedex::verzeichnis::{ordner_lesen, providers_lesen};
use praxishub_thedex::{
    client_new, client_report, DokumentAblegenPayload, Eingang, Nachricht, PatientAnlegenPayload,
    PatientNeuV2, Selbsteintrag, SenderKennung, ThedexFehler, ThedexOrdner,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// THEDEX-Ordner mit Spec-INI und Verzeichnis THEORG.
fn thedex_ordner() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::copy(fixtures().join("THEDEX.INI"), dir.path().join("THEDEX.INI")).unwrap();
    fs::create_dir(dir.path().join("THEORG")).unwrap();
    dir
}

fn kennung() -> SenderKennung {
    SenderKennung {
        verification: Some("9Z9Z".into()),
        ..SenderKennung::praxishub("0.1.0")
    }
}

/// Fixture in den Eingang legen, mit festem Änderungszeitpunkt (Sortierung).
fn ablegen(eingang: &Path, name: &str, sekunden_alt: u64) {
    let ziel = eingang.join(name);
    fs::copy(fixtures().join(name), &ziel).unwrap();
    let t = SystemTime::now() - Duration::from_secs(sekunden_alt);
    fs::File::options()
        .write(true)
        .open(&ziel)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn dateien(ordner: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(ordner)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn selbsteintrag_idempotent_und_schonend() {
    let dir = thedex_ordner();
    let ini = dir.path().join("THEDEX.INI");
    let vorher = fs::read(&ini).unwrap();
    let o = ThedexOrdner::neu(dir.path(), "praxhub").unwrap();
    assert_eq!(o.provider(), "PRAXHUB");

    assert_eq!(
        o.einrichten("Praxishub Connector").unwrap(),
        Selbsteintrag::Neu
    );
    assert!(dir.path().join("PRAXHUB").is_dir());
    let nachher = fs::read(&ini).unwrap();
    let erwartet = [
        &vorher[..vorher.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 2],
        b"PRAXHUB=Praxishub Connector\r\n",
        &vorher[vorher.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 2..],
    ]
    .concat();
    assert_eq!(nachher, erwartet, "nur die eigene Zeile kommt dazu");
    assert!(
        nachher.windows(4).any(|w| w == b"Me\xDF-"),
        "ANSI-Bytes bleiben"
    );

    assert_eq!(
        o.einrichten("Praxishub Connector").unwrap(),
        Selbsteintrag::Unveraendert
    );
    assert_eq!(fs::read(&ini).unwrap(), nachher);
    assert_eq!(
        o.einrichten("Praxishub").unwrap(),
        Selbsteintrag::Aktualisiert
    );
    let text = String::from_utf8_lossy(&fs::read(&ini).unwrap()).into_owned();
    assert_eq!(text.matches("PRAXHUB=").count(), 1);
    assert!(text.contains("[THEORG_PROXOS]\r\nExport=1\r\nCPhoto=1\r\n"));
    assert!(!dir.path().join("PRAXHUB.TMP").exists());

    let p = providers_lesen(dir.path()).unwrap();
    let namen: Vec<_> = p.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(namen, ["THEORG", "MECHTRON", "CYBEX", "BITSOFT", "PRAXHUB"]);
    assert_eq!(
        p[1].beschreibung,
        "mechaTronic DigiMax Meß- und Auswertesoftware"
    );
    assert!(p[0].verzeichnis.is_some());
    assert!(p[1].verzeichnis.is_none());
}

#[test]
fn ohne_ini_kein_thedex_ordner() {
    let dir = tempfile::tempdir().unwrap();
    let o = ThedexOrdner::neu(dir.path(), "PRAXHUB").unwrap();
    assert!(matches!(
        o.einrichten("x"),
        Err(ThedexFehler::KeinThedexOrdner(_))
    ));
    assert!(!dir.path().join("PRAXHUB").exists());
}

#[test]
fn eingang_lesen_ist_nur_lesend() {
    let dir = thedex_ordner();
    let o = ThedexOrdner::neu(dir.path(), "PRAXHUB").unwrap();
    o.einrichten("Praxishub").unwrap();
    ablegen(&o.eingang(), "SPECNEW.TDX", 60);
    ablegen(&o.eingang(), "KAPUTT.TDX", 50);
    let vorher = dateien(&o.eingang());
    let e = o.eingang_lesen().unwrap();
    assert_eq!(e.len(), 2);
    assert!(
        matches!(&e[0], Eingang::Lesbar(d) if d.nachricht.client.first_name.as_deref() == Some("Julia"))
    );
    assert!(matches!(&e[1], Eingang::Defekt { .. }));
    assert_eq!(dateien(&o.eingang()), vorher);
}

#[test]
fn zu_junge_dateien_warten() {
    let dir = thedex_ordner();
    let o = ThedexOrdner::neu(dir.path(), "PRAXHUB").unwrap();
    o.einrichten("Praxishub").unwrap();
    // Gerade erst geschrieben (fs::copy übernimmt auf macOS den alten Zeitstempel).
    fs::write(
        o.eingang().join("SPECNEW.TDX"),
        fs::read(fixtures().join("SPECNEW.TDX")).unwrap(),
    )
    .unwrap();
    assert!(o.eingang_lesen().unwrap().is_empty());
    assert_eq!(ordner_lesen(&o.eingang(), Duration::ZERO).unwrap().len(), 1);
}

#[test]
fn verarbeiten_loescht_erst_nach_quittung() {
    let dir = thedex_ordner();
    let o = ThedexOrdner::neu(dir.path(), "PRAXHUB").unwrap();
    o.einrichten("Praxishub").unwrap();
    let ein = o.eingang();
    ablegen(&ein, "SPECNEW.TDX", 40);
    ablegen(&ein, "KAPUTT.TDX", 30);
    ablegen(&ein, "SPECCHG.TDX", 20);
    ablegen(&ein, "SPECREP.TDX", 10);
    ablegen(&ein, "00010001.bmp", 10);

    // Erste Runde: Cloud nimmt ClientChange nicht an → Abbruch, Rest bleibt.
    let mut gesehen = Vec::new();
    let b = o
        .verarbeiten(|d| {
            gesehen.push(d.pfad.file_name().unwrap().to_string_lossy().into_owned());
            if d.nachricht.client_new_data.is_some() {
                Err("Cloud nicht erreichbar")
            } else {
                Ok(())
            }
        })
        .unwrap();
    assert_eq!(gesehen, ["SPECNEW.TDX", "SPECCHG.TDX"]);
    assert_eq!(b.uebergeben.len(), 1);
    assert_eq!(b.defekt.len(), 1);
    let (pfad, grund) = b.zurueckgestellt.as_ref().unwrap();
    assert!(pfad.ends_with("SPECCHG.TDX"));
    assert_eq!(grund, "Cloud nicht erreichbar");
    assert_eq!(
        dateien(&ein),
        ["00010001.bmp", "FEHLER", "SPECCHG.TDX", "SPECREP.TDX"]
    );
    assert_eq!(
        dateien(&o.fehler_ordner()),
        ["KAPUTT.TDX", "KAPUTT.TDX.TXT"]
    );
    let grund = praxishub_thedex::Zeichensatz::Ansi
        .dekodieren(&fs::read(o.fehler_ordner().join("KAPUTT.TDX.TXT")).unwrap());
    assert!(grund.contains("NUL"), "{grund}");

    // Zweite Runde: alles angenommen; der Anhang verschwindet mit seinem Bericht.
    let mut anhaenge = Vec::new();
    let b = o
        .verarbeiten(|d| {
            anhaenge.push(d.anhang.clone());
            Ok::<(), String>(())
        })
        .unwrap();
    assert_eq!(b.uebergeben.len(), 2);
    assert!(b.zurueckgestellt.is_none());
    assert!(anhaenge[1].as_ref().unwrap().ends_with("00010001.bmp"));
    assert_eq!(dateien(&ein), ["FEHLER"]);

    // Gleichnamige defekte Datei überschreibt die alte in FEHLER nicht.
    ablegen(&ein, "KAPUTT.TDX", 5);
    o.verarbeiten(|_| Ok::<(), String>(())).unwrap();
    assert_eq!(dateien(&o.fehler_ordner()).len(), 4);
}

fn neuanlage() -> PatientAnlegenPayload {
    PatientAnlegenPayload {
        patient: PatientNeuV2 {
            vorname: "Jörg".into(),
            nachname: "Beispiel".into(),
            geburtsdatum: Some("1984-02-29".into()),
            ..Default::default()
        },
        praxishub_patient_id: Some("ph-123".into()),
    }
}

#[test]
fn senden_ohne_verification_schreibt_nichts() {
    let dir = thedex_ordner();
    let o = ThedexOrdner::neu(dir.path(), "PRAXHUB").unwrap();
    let jetzt = Berlin.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap();
    let mut n = client_new(&kennung(), &neuanlage(), "idem-1", jetzt).unwrap();
    n.sender.as_mut().unwrap().verification = None;
    assert!(matches!(
        o.senden("THEORG", &mut n, None),
        Err(ThedexFehler::SenderVerificationFehlt)
    ));
    n.sender.as_mut().unwrap().verification = Some("12".into());
    assert!(o.senden("THEORG", &mut n, None).is_err());
    assert!(dateien(&dir.path().join("THEORG")).is_empty());
}

#[test]
fn senden_atomar_und_8_3() {
    let dir = thedex_ordner();
    let o = ThedexOrdner::neu(dir.path(), "PRAXHUB").unwrap();
    let jetzt = Berlin.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap();
    let mut n = client_new(&kennung(), &neuanlage(), "idem-1", jetzt).unwrap();
    let a = o.senden("theorg", &mut n, None).unwrap();
    let b = o.senden("THEORG", &mut n, None).unwrap();
    assert_ne!(a.tdx, b.tdx);
    let liste = dateien(&dir.path().join("THEORG"));
    assert_eq!(liste.len(), 2);
    for name in &liste {
        let (stamm, endung) = name.split_once('.').unwrap();
        assert_eq!(stamm.len(), 8, "{name}");
        assert!(stamm
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        assert_eq!(endung, "TDX");
    }
    let gelesen = Nachricht::aus_bytes(&fs::read(&a.tdx).unwrap()).unwrap();
    assert_eq!(gelesen, n);
    assert_eq!(gelesen.client.first_name.as_deref(), Some("Jörg"));

    assert!(matches!(
        o.senden("CYBEX", &mut n, None),
        Err(ThedexFehler::ZielUnbekannt(_))
    ));
}

#[test]
fn senden_mit_anhang() {
    let dir = thedex_ordner();
    let o = ThedexOrdner::neu(dir.path(), "PRAXHUB").unwrap();
    let jetzt = Berlin.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap();
    let payload = DokumentAblegenPayload {
        vorname: Some("Erika".into()),
        nachname: Some("Mustermann".into()),
        geburtsdatum: Some("1959-03-17".into()),
        titel: "Anamnesebogen".into(),
        dateiname: "anamnese.pdf".into(),
        mime: "application/pdf".into(),
        inhalt_base64: "JVBERi0xLjQKJcOkw7zDtsOfCg==".into(),
        ..Default::default()
    };
    let (mut n, anhang) = client_report(&kennung(), &payload, None, jetzt).unwrap();
    let g = o.senden("THEORG", &mut n, Some(&anhang)).unwrap();
    let anhang_pfad = g.anhang.unwrap();
    assert_eq!(fs::read(&anhang_pfad).unwrap(), anhang.inhalt);
    let name = anhang_pfad
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.ends_with(".PDF") && name.len() == 12, "{name}");
    let gelesen = Nachricht::aus_bytes(&fs::read(&g.tdx).unwrap()).unwrap();
    let r = gelesen.report_data.unwrap();
    assert_eq!(r.report_file.as_deref(), Some(name.as_str()));
    assert_eq!(r.report_type.as_deref(), Some("PDF"));
    assert_eq!(
        dateien(&dir.path().join("THEORG")).len(),
        2,
        "keine .TMP/.TMA übrig"
    );
}

#[test]
fn probe_lesen_der_fixtures_veraendert_nichts() {
    let vorher = dateien(&fixtures());
    let e = ordner_lesen(&fixtures(), Duration::ZERO).unwrap();
    let defekt = e
        .iter()
        .filter(|x| matches!(x, Eingang::Defekt { .. }))
        .count();
    assert_eq!(defekt, 3, "KAPUTT, NOMSG, NOCLIENT");
    let rep = e
        .iter()
        .find_map(|x| match x {
            Eingang::Lesbar(d) if d.pfad.ends_with("SPECREP.TDX") => Some(d),
            _ => None,
        })
        .unwrap();
    assert!(rep.anhang.as_ref().unwrap().ends_with("00010001.bmp"));
    assert_eq!(dateien(&fixtures()), vorher);
}
