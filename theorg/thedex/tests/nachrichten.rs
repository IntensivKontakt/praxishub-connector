//! Beispiele der Spezifikation und Grenzfälle: Lesen, Rundreise, Abbildung auf v2.

use praxishub_thedex::modell::Nachrichtentyp;
use praxishub_thedex::{abbilden, AbbildungsOptionen, Nachricht, ThedexFehler, Zeichensatz};
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn lesen(name: &str) -> Nachricht {
    Nachricht::aus_bytes(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

const LESBAR: &[&str] = &[
    "SPECNEW.TDX",
    "SPECCHG.TDX",
    "SPECREP.TDX",
    "SPECCIO.TDX",
    "SPECRDA.TDX",
    "SPECBIL.TDX",
    "VERTRAG.TDX",
    "VERTRAG2.TDX",
    "OEMNEW.TDX",
    "DELETE.TDX",
    "UNBEK.TDX",
    "NOKOPF.TDX",
];

#[test]
fn rundreise_aller_lesbaren_dateien() {
    for name in LESBAR {
        let n = lesen(name);
        let bytes = n.als_bytes();
        let wieder = Nachricht::aus_bytes(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(wieder, n, "{name}: Bytes-Rundreise");
        assert_eq!(
            Nachricht::aus_text(&n.als_text()).unwrap(),
            n,
            "{name}: Text-Rundreise"
        );
        // Zweites Schreiben ist byte-identisch (stabile Ausgabe).
        assert_eq!(wieder.als_bytes(), bytes, "{name}: stabil");
    }
}

#[test]
fn kanonische_dateien_bleiben_byte_identisch() {
    for name in ["OEMNEW.TDX", "DELETE.TDX", "VERTRAG.TDX", "SPECRDA.TDX"] {
        assert_eq!(lesen(name).als_bytes(), fixture(name), "{name}");
    }
}

#[test]
fn spec_client_new() {
    let n = lesen("SPECNEW.TDX");
    assert_eq!(n.typ(), Nachrichtentyp::ClientNew);
    assert_eq!(n.zeichensatz(), Zeichensatz::Ansi);
    assert_eq!(
        n.thedex.as_ref().unwrap().version_number.as_deref(),
        Some("2.1")
    );
    let s = n.sender.as_ref().unwrap();
    assert_eq!(s.identification.as_deref(), Some("THEORG"));
    assert_eq!(s.verification.as_deref(), Some("02A7"));
    let c = &n.client;
    // `CTelephon` aus dem Beispiel landet im richtigen Feld.
    assert_eq!(c.telephone.as_deref(), Some(""));
    assert!(c.weitere.is_empty(), "{:?}", c.weitere);
    assert_eq!(c.sex.as_deref(), Some("F"));
    assert_eq!(c.bank_identification_code.as_deref(), Some("10010000"));
    let ud = c.user_defined.as_ref().unwrap();
    assert_eq!(ud.len(), 6);
    assert_eq!(ud[1], "Schmerzpatient");

    let ab = abbilden(&n, &AbbildungsOptionen::default());
    assert!(ab.hinweise.is_empty(), "{:?}", ab.hinweise);
    assert_eq!(ab.patienten.len(), 1);
    let p = &ab.patienten[0];
    assert_eq!(p.pvs_id, "1166");
    assert!(!p.geloescht);
    assert_eq!(p.vorname.as_deref(), Some("Julia"));
    assert_eq!(p.nachname.as_deref(), Some("Mustermann"));
    assert_eq!(p.geburtsdatum.as_deref(), Some("1965-01-11"));
    assert_eq!(p.geschlecht.as_deref(), Some("w"));
    assert_eq!(p.telefon, None);
    assert_eq!(p.telefon_mobil.as_deref(), Some("+49172875569"));
    assert_eq!(p.email.as_deref(), Some("Julia.Mustermann@t-online.de"));
    assert_eq!(p.strasse.as_deref(), Some("Musterstr. 11"));
    assert_eq!(p.plz.as_deref(), Some("99999"));
    assert_eq!(p.ort.as_deref(), Some("Musterhausen"));
    assert_eq!(p.kostentraeger.as_deref(), Some("gkv"));
    assert_eq!(p.checkin_id.as_deref(), Some("200031969"));
    assert_eq!(p.externe_id, None);
    assert_eq!(p.geaendert_am.as_deref(), Some("2014-09-11T09:52:00+02:00"));
    assert!(ab.mitgliedschaften.is_empty() && ab.checkins.is_empty());
}

#[test]
fn patient_json_feldnamen_wie_im_vertrag() {
    let ab = abbilden(&lesen("SPECNEW.TDX"), &AbbildungsOptionen::default());
    let v = serde_json::to_value(&ab.patienten[0]).unwrap();
    let mut schluessel: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    schluessel.sort();
    assert_eq!(
        schluessel,
        [
            "checkin_id",
            "email",
            "geaendert_am",
            "geburtsdatum",
            "geloescht",
            "geschlecht",
            "kostentraeger",
            "nachname",
            "ort",
            "plz",
            "pvs_id",
            "strasse",
            "telefon_mobil",
            "vorname"
        ]
    );
}

#[test]
fn spec_client_change_nimmt_neue_daten() {
    let n = lesen("SPECCHG.TDX");
    assert_eq!(n.typ(), Nachrichtentyp::ClientChange);
    assert_eq!(n.client.street.as_deref(), Some("Musterstr. 11"));
    assert_eq!(
        n.client.user_defined.as_ref().unwrap()[1],
        "Herzschwäche",
        "Umlaut aus Windows-1252"
    );
    let ab = abbilden(&n, &AbbildungsOptionen::default());
    let p = &ab.patienten[0];
    assert_eq!(p.strasse.as_deref(), Some("Marktplatz 17"));
    assert_eq!(p.plz.as_deref(), Some("99998"));
    assert_eq!(p.ort.as_deref(), Some("Musterdorf"));
    assert_eq!(p.telefon.as_deref(), Some("09999/1234567"));
    assert_eq!(p.kostentraeger.as_deref(), Some("gkv"));
}

#[test]
fn spec_client_report_fremder_sender_wird_nicht_abgebildet() {
    let n = lesen("SPECREP.TDX");
    assert_eq!(n.typ(), Nachrichtentyp::ClientReport);
    let s = n.sender.as_ref().unwrap();
    assert_eq!(
        s.lan_identification.as_deref(),
        Some("Deutschland"),
        "Tippfehler SenderLanIdetifikation"
    );
    assert_eq!(
        s.description.as_deref(),
        Some("DigiMax Meß- und Auswertesoftware")
    );
    assert_eq!(n.client.user_defined.as_ref().unwrap()[0], "Rückenschule");
    let r = n.report_data.as_ref().unwrap();
    assert_eq!(r.report_type.as_deref(), Some("BMP"));
    assert_eq!(r.report_file.as_deref(), Some("00010001.bmp"));
    assert_eq!(r.report_therapist.as_deref(), Some(""));

    let ab = abbilden(&n, &AbbildungsOptionen::default());
    assert!(ab.ist_leer());
    assert!(ab.hinweise[0].contains("mechaTronic"), "{:?}", ab.hinweise);

    // Mit mechaTronic als PVS-Provider (nur zum Test) wird abgebildet.
    let opt = AbbildungsOptionen {
        pvs_provider: "MECHATRONIC".into(),
    };
    let ab = abbilden(&n, &opt);
    assert_eq!(ab.patienten[0].pvs_id, "339");
    assert_eq!(ab.patienten[0].externe_id.as_deref(), Some("51-781451-78"));
}

#[test]
fn spec_check_out() {
    let n = lesen("SPECCIO.TDX");
    assert_eq!(n.typ(), Nachrichtentyp::ClientCheckInOut);
    let ab = abbilden(&n, &AbbildungsOptionen::default());
    assert!(ab.hinweise.is_empty(), "{:?}", ab.hinweise);
    assert_eq!(ab.patienten[0].pvs_id, "339");
    assert_eq!(ab.patienten[0].geschlecht.as_deref(), Some("w"));
    assert_eq!(ab.checkins.len(), 1);
    let c = &ab.checkins[0];
    assert_eq!(c.pvs_id, "339:out:201409112048");
    assert_eq!(c.patient_pvs_id, "339");
    assert_eq!(c.ereignis, "out");
    assert_eq!(c.zeitpunkt, "2014-09-11T20:48:00+02:00");
    assert_eq!(c.notiz.as_deref(), Some("Vertrag 36 Monate"));
    let v = serde_json::to_value(c).unwrap();
    for k in [
        "pvs_id",
        "patient_pvs_id",
        "ereignis",
        "zeitpunkt",
        "notiz",
        "geloescht",
    ] {
        assert!(v.get(k).is_some(), "{k}");
    }
}

#[test]
fn spec_report_data_beispiel() {
    let n = lesen("SPECRDA.TDX");
    let r = n.report_data.as_ref().unwrap();
    assert_eq!(
        r.report_description.as_deref(),
        Some("Trainingsbericht ErgoBike")
    );
    assert_eq!(r.report_therapist.as_deref(), Some("Heike"));
    assert_eq!(r.report_file.as_deref(), Some("REP00001.BMP"));
}

#[test]
fn bilaterale_abschnitte_bleiben_erhalten() {
    let n = lesen("SPECBIL.TDX");
    let namen: Vec<_> = n
        .weitere_abschnitte
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(namen, ["Cybex", "mechaTronic"]);
    assert_eq!(n.weitere_abschnitte[0].wert("patientid"), Some("933 TH"));
    assert_eq!(n.weitere_abschnitte[0].felder.len(), 18);
    let ud = n.client.user_defined.as_ref().unwrap();
    assert_eq!(
        ud,
        &[
            "Patient hat starke Schmerzen",
            "Hautallergie",
            "",
            "",
            "Sportler",
            ""
        ]
    );
    let text = n.als_text();
    assert!(text.contains("[Cybex]\r\nPatientID=933 TH\r\n"));
    assert!(text.contains("UserDefined=6\r\nUserDefined1=Patient hat starke Schmerzen\r\n"));
}

#[test]
fn vertrag_wird_mitgliedschaft() {
    let ab = abbilden(&lesen("VERTRAG.TDX"), &AbbildungsOptionen::default());
    let p = &ab.patienten[0];
    assert_eq!(p.vorname.as_deref(), Some("Jörg"));
    assert_eq!(p.geburtsdatum.as_deref(), Some("1984-02-29"));
    assert_eq!(p.kostentraeger.as_deref(), Some("selbstzahler"));
    assert_eq!(p.checkin_id.as_deref(), Some("CHIP-0815"));
    assert_eq!(ab.mitgliedschaften.len(), 1);
    let m = &ab.mitgliedschaften[0];
    assert_eq!(m.patient_pvs_id, "2001");
    assert!(m.pvs_id.starts_with("2001:20140101:"));
    assert!(m.pvs_id.len() <= 128);
    assert_eq!(m.bezeichnung.as_deref(), Some("Gerätetraining 24 Monate"));
    assert_eq!(m.beginn.as_deref(), Some("2014-01-01"));
    assert_eq!(m.ende.as_deref(), Some("2015-12-31"));
    assert_eq!(m.kuendigung_zum.as_deref(), Some("2015-09-30"));
    assert_eq!(m.ruhezeit_von.as_deref(), Some("2014-07-01"));
    assert_eq!(m.ruhezeit_bis.as_deref(), Some("2014-07-31"));
    assert!(!m.geloescht);
    // Deterministisch: zweite Abbildung derselben Datei ergibt dieselbe Kennung.
    let ab2 = abbilden(&lesen("VERTRAG.TDX"), &AbbildungsOptionen::default());
    assert_eq!(ab2.mitgliedschaften[0].pvs_id, m.pvs_id);
    let v = serde_json::to_value(m).unwrap();
    for k in [
        "pvs_id",
        "patient_pvs_id",
        "bezeichnung",
        "beginn",
        "ende",
        "kuendigung_zum",
        "ruhezeit_von",
        "ruhezeit_bis",
        "geloescht",
    ] {
        assert!(v.get(k).is_some(), "{k}");
    }
}

#[test]
fn mehrere_vertragsabschnitte() {
    let n = lesen("VERTRAG2.TDX");
    assert_eq!(n.contract_data.len(), 3);
    let ab = abbilden(&n, &AbbildungsOptionen::default());
    assert_eq!(ab.mitgliedschaften.len(), 2);
    assert_ne!(ab.mitgliedschaften[0].pvs_id, ab.mitgliedschaften[1].pvs_id);
    assert_eq!(
        ab.mitgliedschaften[0].bezeichnung.as_deref(),
        Some("Rückenkurs")
    );
    assert!(ab.hinweise.iter().any(|h| h.contains("ohne Inhalt")));
    assert_eq!(ab.patienten[0].kostentraeger, None);
}

#[test]
fn oem_umlaute() {
    let bytes = fixture("OEMNEW.TDX");
    assert!(
        bytes.windows(4).any(|w| w == b"M\x81\xE1i"),
        "Fixture ist CP850"
    );
    let n = lesen("OEMNEW.TDX");
    assert_eq!(n.zeichensatz(), Zeichensatz::Oem);
    assert_eq!(n.client.last_name.as_deref(), Some("Müßig"));
    assert_eq!(n.client.first_name.as_deref(), Some("Jürgen"));
    assert_eq!(n.client.street.as_deref(), Some("Hauptstraße 5"));
    assert_eq!(n.client.town.as_deref(), Some("Köln"));
    // Als ANSI gelesen wären die Umlaute falsch.
    let falsch = Nachricht::aus_text(&Zeichensatz::Ansi.dekodieren(&bytes)).unwrap();
    assert_ne!(falsch.client.last_name.as_deref(), Some("Müßig"));
}

#[test]
fn loeschung() {
    let ab = abbilden(&lesen("DELETE.TDX"), &AbbildungsOptionen::default());
    assert_eq!(ab.patienten.len(), 1);
    assert!(ab.patienten[0].geloescht);
    assert_eq!(ab.patienten[0].pvs_id, "1166");
}

#[test]
fn unbekannte_und_leere_felder() {
    let n = lesen("UNBEK.TDX");
    assert_eq!(n.typ(), Nachrichtentyp::ClientInfo);
    assert_eq!(
        n.zeichensatz(),
        Zeichensatz::Ansi,
        "fehlender Zeichensatz = ANSI"
    );
    let t = n.thedex.as_ref().unwrap();
    assert_eq!(t.version_number.as_deref(), Some("2.1"));
    assert_eq!(t.weitere[0].schluessel, "ThedexBuild");
    assert_eq!(n.message.weitere[0].schluessel, "MessageExtra");
    let c = &n.client;
    assert_eq!(c.first_name.as_deref(), Some(""));
    assert_eq!(c.telephone.as_deref(), Some("030 1234567"));
    assert_eq!(c.weitere.len(), 1);
    assert_eq!(c.weitere[0].wert, "Bär");
    assert_eq!(c.user_defined.as_ref().unwrap(), &["", "", "nur drei"]);
    assert_eq!(n.weitere_abschnitte[0].name, "THEORG");

    let ab = abbilden(&n, &AbbildungsOptionen::default());
    let p = &ab.patienten[0];
    assert_eq!(p.pvs_id, "4004");
    assert_eq!(p.vorname, None, "leere Felder werden weggelassen");
    assert_eq!(p.nachname.as_deref(), Some("Leerfeld"));
    assert_eq!(p.geburtsdatum, None);
    assert_eq!(p.geschlecht, None);
    assert_eq!(p.geaendert_am.as_deref(), Some("2015-03-04T07:05:00+01:00"));
}

#[test]
fn fehlende_pflichtabschnitte() {
    for (name, teil) in [("NOMSG.TDX", "[Message]"), ("NOCLIENT.TDX", "[Client]")] {
        match Nachricht::aus_bytes(&fixture(name)) {
            Err(ThedexFehler::Format(f)) => assert!(f.contains(teil), "{name}: {f}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    // [Thedex]/[Sender] fehlen: lesbar, aber gemeldet; ohne Sender keine pvs_id.
    let n = lesen("NOKOPF.TDX");
    assert_eq!(n.maengel().len(), 2);
    let ab = abbilden(&n, &AbbildungsOptionen::default());
    assert!(ab.ist_leer());
    assert_eq!(ab.hinweise.len(), 3, "{:?}", ab.hinweise);
}

#[test]
fn kaputte_datei() {
    let e = Nachricht::aus_bytes(&fixture("KAPUTT.TDX")).unwrap_err();
    assert_eq!(e.fehlercode(), "ungueltig");
    assert!(Nachricht::aus_bytes(b"irgendein Text ohne Abschnitte").is_err());
    assert!(Nachricht::aus_bytes(b"").is_err());
}

#[test]
fn utf8_mit_bom_wird_erkannt() {
    let text = "\u{FEFF}[Message]\r\nMessageType=ClientInfo\r\n[Client]\r\nCLastName=Weiß\r\n";
    let n = Nachricht::aus_bytes(text.as_bytes()).unwrap();
    assert_eq!(n.client.last_name.as_deref(), Some("Weiß"));
}
