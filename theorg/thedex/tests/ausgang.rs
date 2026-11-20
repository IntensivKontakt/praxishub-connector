//! Cloud-Aufträge → THEDEX-Nachrichten an THEORG.

use chrono::TimeZone;
use chrono_tz::Europe::Berlin;
use praxishub_thedex::modell::Nachrichtentyp;
use praxishub_thedex::{
    client_new, client_report, DokumentAblegenPayload, Nachricht, PatientAnlegenPayload, PatientV2,
    SenderKennung,
};

fn kennung() -> SenderKennung {
    SenderKennung {
        verification: Some("9Z9Z".into()),
        ..SenderKennung::praxishub("0.1.0")
    }
}

fn jetzt() -> chrono::DateTime<chrono_tz::Tz> {
    Berlin.with_ymd_and_hms(2026, 11, 20, 9, 5, 0).unwrap()
}

#[test]
fn patient_anlegen_wird_client_new() {
    // Payload so, wie die Cloud ihn laut Vertrag 6.1 schickt.
    let payload: PatientAnlegenPayload = serde_json::from_str(
        r#"{"patient": {"vorname": "Jürgen", "nachname": "Müßig", "geburtsdatum": "1970-06-05",
            "geschlecht": "m", "telefon_mobil": "+49 170 0000000", "email": "j@example.org",
            "strasse": "Hauptstraße 5", "plz": "50667", "ort": "Köln", "kostentraeger": "selbstzahler"},
            "praxishub_patient_id": "4711", "unbekannt": true}"#,
    )
    .unwrap();
    let n = client_new(&kennung(), &payload, "job-abc", jetzt()).unwrap();
    assert_eq!(n.typ(), Nachrichtentyp::ClientNew);
    let text = n.als_text();
    let erwartet = "[Thedex]\r\nThedexVersionNumber=2.1\r\n\r\n\
[Sender]\r\nSenderIdentification=PRAXHUB\r\nSenderVerification=9Z9Z\r\nSenderDescription=Praxishub Connector\r\nSenderSoftwareVersionInformation=0.1.0\r\nSenderLanIdentification=PRAXHUB\r\n\r\n\
[Message]\r\nMessageType=ClientNew\r\nMessageCountryInformation=049\r\nMessageCodePageInformation=ANSI\r\nMessageDate=20112026\r\nMessageTime=09:05\r\n\r\n\
[Client]\r\nCExternalIdentification=4711\r\nCLastName=Müßig\r\nCFirstName=Jürgen\r\nCBirthday=05061970\r\nCSex=M\r\nCStreet=Hauptstraße 5\r\nCZip=50667\r\nCTown=Köln\r\nInsuranceStatus=P\r\neMail=j@example.org\r\nCellularPhone=+49 170 0000000\r\n";
    assert_eq!(text, erwartet);
    let bytes = n.als_bytes();
    assert!(
        bytes.windows(5).any(|w| w == b"M\xFC\xDFig"),
        "Windows-1252"
    );
    assert_eq!(Nachricht::aus_bytes(&bytes).unwrap(), n);
}

#[test]
fn patient_anlegen_ohne_praxishub_id_nimmt_idempotenz_schluessel() {
    let mut payload = PatientAnlegenPayload::default();
    payload.patient.vorname = "A".into();
    payload.patient.nachname = "B".into();
    payload.patient.geschlecht = Some("d".into());
    payload.patient.kostentraeger = Some("gkv".into());
    let n = client_new(&kennung(), &payload, "job-abc", jetzt()).unwrap();
    assert_eq!(n.client.external_identification.as_deref(), Some("job-abc"));
    assert_eq!(n.client.sex, None);
    assert_eq!(n.client.insurance_status, None);
    assert_eq!(n.client.birthday, None);
}

#[test]
fn patient_anlegen_fehler() {
    let mut payload = PatientAnlegenPayload::default();
    payload.patient.vorname = "A".into();
    let e = client_new(&kennung(), &payload, "job", jetzt()).unwrap_err();
    assert_eq!(e.fehlercode(), "ungueltig");
    assert!(!e.wiederholbar());

    payload.patient.nachname = "B".into();
    payload.patient.geburtsdatum = Some("05.06.1970".into());
    assert_eq!(
        client_new(&kennung(), &payload, "job", jetzt())
            .unwrap_err()
            .fehlercode(),
        "ungueltig"
    );

    payload.patient.geburtsdatum = None;
    let ohne = SenderKennung::praxishub("0.1.0");
    assert_eq!(
        client_new(&ohne, &payload, "job", jetzt()).unwrap_err().to_string(),
        "SenderVerification fehlt oder ist nicht vierstellig – ohne Kennung von SOVDWAER wird nichts geschrieben"
    );
}

fn dokument() -> DokumentAblegenPayload {
    serde_json::from_str(
        r#"{"patient_pvs_id": "339", "titel": "Befund Röntgen", "datum": "2026-11-19T16:30:00+00:00",
            "dateiname": "befund.pdf", "mime": "application/pdf", "inhalt_base64": "JVBERi0xLjQK",
            "therapeut": "Heike"}"#,
    )
    .unwrap()
}

fn erika() -> PatientV2 {
    PatientV2 {
        pvs_id: "339".into(),
        vorname: Some("Erika".into()),
        nachname: Some("Mustermann".into()),
        geburtsdatum: Some("1959-03-17".into()),
        externe_id: Some("51-781451-78".into()),
        ..Default::default()
    }
}

#[test]
fn dokument_ablegen_wird_client_report() {
    let (n, anhang) = client_report(&kennung(), &dokument(), Some(&erika()), jetzt()).unwrap();
    assert_eq!(n.typ(), Nachrichtentyp::ClientReport);
    assert_eq!(anhang.endung, "PDF");
    assert_eq!(anhang.inhalt, b"%PDF-1.4\n");
    let c = &n.client;
    assert_eq!(c.last_name.as_deref(), Some("Mustermann"));
    assert_eq!(c.first_name.as_deref(), Some("Erika"));
    assert_eq!(c.birthday.as_deref(), Some("17031959"));
    assert_eq!(c.external_identification.as_deref(), Some("51-781451-78"));
    assert_eq!(c.sender_internal_identification, None);
    let r = n.report_data.as_ref().unwrap();
    // 16:30 UTC = 17:30 Berliner Winterzeit
    assert_eq!(r.report_date.as_deref(), Some("19112026"));
    assert_eq!(r.report_time.as_deref(), Some("17:30"));
    assert_eq!(r.report_description.as_deref(), Some("Befund Röntgen"));
    assert_eq!(r.report_therapist.as_deref(), Some("Heike"));
    assert_eq!(r.report_type.as_deref(), Some("PDF"));
    assert_eq!(r.report_file, None, "setzt erst senden()");
}

#[test]
fn dokument_ablegen_patientenangaben() {
    // Ohne Namen und ohne bekannten Patienten: THEORG kann nicht zuordnen.
    let e = client_report(&kennung(), &dokument(), None, jetzt()).unwrap_err();
    assert_eq!(e.fehlercode(), "patient_unbekannt");

    // Bekannter Patient mit anderer pvs_id zählt nicht.
    let mut fremd = erika();
    fremd.pvs_id = "340".into();
    assert!(client_report(&kennung(), &dokument(), Some(&fremd), jetzt()).is_err());

    // Angaben im Payload genügen; Datum ohne Uhrzeit → 00:00.
    let mut p = dokument();
    p.patient_pvs_id = None;
    p.vorname = Some("Max".into());
    p.nachname = Some("Mustermann".into());
    p.geburtsdatum = Some("1970-02-01".into());
    p.datum = Some("2026-11-18".into());
    p.therapeut = None;
    let (n, _) = client_report(&kennung(), &p, None, jetzt()).unwrap();
    let r = n.report_data.as_ref().unwrap();
    assert_eq!(r.report_date.as_deref(), Some("18112026"));
    assert_eq!(r.report_time.as_deref(), Some("00:00"));
    assert_eq!(r.report_therapist, None);
    assert_eq!(n.client.birthday.as_deref(), Some("01021970"));

    // Ohne datum gilt der Sendezeitpunkt.
    p.datum = None;
    let (n, _) = client_report(&kennung(), &p, None, jetzt()).unwrap();
    assert_eq!(n.report_data.unwrap().report_time.as_deref(), Some("09:05"));
}

#[test]
fn dokument_ablegen_fehler() {
    let mut p = dokument();
    p.mime = "image/png".into();
    p.dateiname = "bild.png".into();
    let e = client_report(&kennung(), &p, Some(&erika()), jetzt()).unwrap_err();
    assert_eq!(e.fehlercode(), "nicht_unterstuetzt");

    let mut p = dokument();
    p.inhalt_base64 = "kein base64!".into();
    let e = client_report(&kennung(), &p, Some(&erika()), jetzt()).unwrap_err();
    assert_eq!(e.fehlercode(), "ungueltig");

    let mut p = dokument();
    p.inhalt_base64 = String::new();
    assert!(client_report(&kennung(), &p, Some(&erika()), jetzt()).is_err());

    let mut p = dokument();
    p.datum = Some("gestern".into());
    assert_eq!(
        client_report(&kennung(), &p, Some(&erika()), jetzt())
            .unwrap_err()
            .fehlercode(),
        "ungueltig"
    );
}
