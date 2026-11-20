//! Ausgehende Nachrichten an THEORG aus Cloud-Aufträgen.
//!
//! * `patient_anlegen` (Vertrag 6.1) → `ClientNew`
//! * `dokument_ablegen` (neu)        → `ClientReport` mit Anhangsdatei
//!
//! Geschrieben werden nur Felder mit Inhalt: THEORG kann so eingestellt sein,
//! dass leere Felder vorhandene Inhalte löschen („Leerinformationen löschen
//! Feldinhalte“). Die Dateien selbst schreibt [`crate::ThedexOrdner::senden`].

use crate::fehler::{Ergebnis, ThedexFehler};
use crate::modell::{Berichtsdaten, Nachricht, Nachrichtentyp, Person, Sender};
use crate::v2::{berlin, PatientV2};
use crate::verzeichnis::providername_pruefen;
use crate::werte::{datum_lesen, datum_schreiben, zeit_schreiben};
use base64::Engine;
use chrono::{DateTime, NaiveDate};
use chrono_tz::{Europe::Berlin, Tz};
use serde::{Deserialize, Serialize};

/// Sender-Kennung von Praxishub im THEDEX-System.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderKennung {
    /// `SenderIdentification` = Providername = eigenes Verzeichnis (≤ 8 Zeichen).
    pub identification: String,
    /// `SenderVerification`, vierstellig, vergibt SOVDWAER. Ohne sie wird nichts geschrieben.
    pub verification: Option<String>,
    /// `SenderDescription`
    pub description: String,
    /// `SenderSoftwareVersionInformation`
    pub software_version: String,
}

impl SenderKennung {
    /// Vorschlag für Praxishub; `verification` bleibt leer, bis SOVDWAER sie vergibt.
    pub fn praxishub(software_version: impl Into<String>) -> Self {
        SenderKennung {
            identification: crate::PROVIDER_STANDARD.to_string(),
            verification: None,
            description: "Praxishub Connector".to_string(),
            software_version: software_version.into(),
        }
    }

    /// Prüft Providername und Verification.
    pub fn pruefen(&self) -> Ergebnis<()> {
        providername_pruefen(&self.identification)?;
        match self.verification.as_deref() {
            Some(v) if verification_gueltig(v) => Ok(()),
            _ => Err(ThedexFehler::SenderVerificationFehlt),
        }
    }

    /// `[Sender]`-Abschnitt. `SenderLanIdentification` = Providername.
    pub fn sender_abschnitt(&self) -> Sender {
        let provider = self.identification.trim().to_ascii_uppercase();
        Sender {
            identification: Some(provider.clone()),
            verification: self.verification.as_ref().map(|v| v.trim().to_string()),
            description: Some(self.description.clone()),
            software_version_information: Some(self.software_version.clone()),
            lan_identification: Some(provider),
            weitere: Vec::new(),
        }
    }
}

/// Vier Zeichen, Ziffern/Buchstaben (Beispiele der Spezifikation: `02A7`, `038D`).
pub fn verification_gueltig(v: &str) -> bool {
    let v = v.trim();
    v.len() == 4 && v.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Nachrichtengerüst mit `[Thedex]`, `[Sender]` und `[Message]` (Datum/Zeit aus `jetzt`).
pub fn geruest(
    typ: Nachrichtentyp,
    kennung: &SenderKennung,
    jetzt: DateTime<Tz>,
) -> Ergebnis<Nachricht> {
    kennung.pruefen()?;
    let mut n = Nachricht::neu(typ);
    n.sender = Some(kennung.sender_abschnitt());
    let lokal = jetzt.with_timezone(&Berlin);
    n.message.date = Some(datum_schreiben(lokal.date_naive()));
    n.message.time = Some(zeit_schreiben(lokal.time()));
    Ok(n)
}

// ------------------------------------------------------------ patient_anlegen

/// `PatientNeuV2` laut Vertrag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatientNeuV2 {
    pub vorname: String,
    pub nachname: String,
    #[serde(default)]
    pub geburtsdatum: Option<String>,
    #[serde(default)]
    pub geschlecht: Option<String>,
    #[serde(default)]
    pub telefon_mobil: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub strasse: Option<String>,
    #[serde(default)]
    pub plz: Option<String>,
    #[serde(default)]
    pub ort: Option<String>,
    #[serde(default)]
    pub kostentraeger: Option<String>,
}

/// `PatientAnlegenPayload` laut Vertrag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatientAnlegenPayload {
    pub patient: PatientNeuV2,
    #[serde(default)]
    pub praxishub_patient_id: Option<String>,
}

/// Wert für `CExternalIdentification` einer Neuanlage: die Praxishub-Patienten-ID,
/// sonst der Idempotenz-Schlüssel des Auftrags. THEORG speichert ihn in der
/// Personenkartei und gibt ihn beim Export wieder mit (→ `PatientV2.externe_id`).
pub fn externe_kennung(payload: &PatientAnlegenPayload, idempotenz_schluessel: &str) -> String {
    payload
        .praxishub_patient_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(idempotenz_schluessel.trim())
        .to_string()
}

/// Auftrag `patient_anlegen` → `ClientNew` an THEORG.
pub fn client_new(
    kennung: &SenderKennung,
    payload: &PatientAnlegenPayload,
    idempotenz_schluessel: &str,
    jetzt: DateTime<Tz>,
) -> Ergebnis<Nachricht> {
    let p = &payload.patient;
    if p.vorname.trim().is_empty() || p.nachname.trim().is_empty() {
        return Err(ThedexFehler::Auftrag(
            "vorname und nachname sind Pflicht".into(),
        ));
    }
    let extern_id = externe_kennung(payload, idempotenz_schluessel);
    if extern_id.is_empty() {
        return Err(ThedexFehler::Auftrag(
            "weder praxishub_patient_id noch idempotenz_schluessel".into(),
        ));
    }
    let geburtsdatum = optional_datum(p.geburtsdatum.as_deref(), "geburtsdatum")?;

    let mut n = geruest(Nachrichtentyp::ClientNew, kennung, jetzt)?;
    n.client = Person {
        external_identification: Some(extern_id),
        last_name: Some(p.nachname.trim().to_string()),
        first_name: Some(p.vorname.trim().to_string()),
        birthday: geburtsdatum.map(datum_schreiben),
        sex: p.geschlecht.as_deref().and_then(csex),
        street: text(&p.strasse),
        zip: text(&p.plz),
        town: text(&p.ort),
        cellular_phone: text(&p.telefon_mobil),
        email: text(&p.email),
        insurance_status: p.kostentraeger.as_deref().and_then(insurance_status),
        ..Default::default()
    };
    Ok(n)
}

/// `m`/`w` → `M`/`W`. `d` hat in THEDEX keinen Wert und bleibt weg.
fn csex(g: &str) -> Option<String> {
    match g.trim().to_ascii_lowercase().as_str() {
        "m" => Some("M".into()),
        "w" => Some("W".into()),
        _ => None,
    }
}

/// `selbstzahler`/`pkv` → `P`; für GKV fehlt die IK-Nummer, THEORG setzt dann Status 1.
fn insurance_status(k: &str) -> Option<String> {
    match k.trim() {
        "selbstzahler" | "pkv" => Some("P".into()),
        _ => None,
    }
}

// ------------------------------------------------------------ dokument_ablegen

/// Payload des neuen Auftrags `dokument_ablegen`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DokumentAblegenPayload {
    #[serde(default)]
    pub patient_pvs_id: Option<String>,
    #[serde(default)]
    pub vorname: Option<String>,
    #[serde(default)]
    pub nachname: Option<String>,
    /// `JJJJ-MM-TT`
    #[serde(default)]
    pub geburtsdatum: Option<String>,
    pub titel: String,
    /// `JJJJ-MM-TT` oder Zeitpunkt ISO 8601; fehlt er, gilt der Sendezeitpunkt.
    #[serde(default)]
    pub datum: Option<String>,
    pub dateiname: String,
    pub mime: String,
    pub inhalt_base64: String,
    #[serde(default)]
    pub therapeut: Option<String>,
}

/// Anhangsdatei eines Berichts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anhang {
    pub inhalt: Vec<u8>,
    /// Dateiendung ohne Punkt, groß (`PDF`, `JPG` …).
    pub endung: String,
}

/// `ReportType` aus MIME-Typ, ersatzweise aus der Dateiendung.
pub fn report_type(mime: &str, dateiname: &str) -> Option<&'static str> {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let aus_mime = match mime.as_str() {
        "application/pdf" => Some("PDF"),
        "image/jpeg" | "image/jpg" | "image/pjpeg" => Some("JPG"),
        "text/plain" => Some("TXT"),
        "application/rtf" | "text/rtf" => Some("RTF"),
        "application/msword" => Some("DOC"),
        "application/vnd.ms-excel" => Some("XLS"),
        "text/html" => Some("HTM"),
        "image/bmp" | "image/x-bmp" | "image/x-ms-bmp" => Some("BMP"),
        "image/x-pcx" | "image/pcx" => Some("PCX"),
        _ => None,
    };
    aus_mime.or_else(|| {
        let endung = dateiname.rsplit_once('.')?.1.to_ascii_uppercase();
        match endung.as_str() {
            "PDF" => Some("PDF"),
            "JPG" | "JPEG" => Some("JPG"),
            "TXT" => Some("TXT"),
            "RTF" => Some("RTF"),
            "DOC" => Some("DOC"),
            "XLS" => Some("XLS"),
            "HTM" | "HTML" => Some("HTM"),
            "BMP" => Some("BMP"),
            "PCX" => Some("PCX"),
            _ => None,
        }
    })
}

/// Auftrag `dokument_ablegen` → `ClientReport` plus Anhang.
///
/// THEORG ordnet eingehende Personen über Nachname, Vorname und Geburtsdatum
/// zu; die eigene Personennummer wertet es beim Import nicht aus. Fehlen diese
/// Angaben im Payload, nimmt die Funktion sie aus `bekannt` (dem zuletzt aus
/// THEORG gelesenen Patienten zu `patient_pvs_id`, falls der Aufrufer ihn hat).
/// `ReportFile` setzt erst [`crate::ThedexOrdner::senden`].
pub fn client_report(
    kennung: &SenderKennung,
    payload: &DokumentAblegenPayload,
    bekannt: Option<&PatientV2>,
    jetzt: DateTime<Tz>,
) -> Ergebnis<(Nachricht, Anhang)> {
    if payload.titel.trim().is_empty() {
        return Err(ThedexFehler::Auftrag("titel fehlt".into()));
    }
    let typ = report_type(&payload.mime, &payload.dateiname).ok_or_else(|| {
        ThedexFehler::NichtUnterstuetzt(format!(
            "Dateityp {:?} / {:?} hat keinen THEDEX-ReportType",
            payload.mime, payload.dateiname
        ))
    })?;
    let inhalt = base64::engine::general_purpose::STANDARD
        .decode(payload.inhalt_base64.trim())
        .map_err(|e| ThedexFehler::Auftrag(format!("inhalt_base64: {e}")))?;
    if inhalt.is_empty() {
        return Err(ThedexFehler::Auftrag("Dokument ist leer".into()));
    }

    // Bekannter Patient zählt nur, wenn er zur angefragten pvs_id passt.
    let bekannt = bekannt.filter(|b| {
        payload
            .patient_pvs_id
            .as_deref()
            .is_some_and(|id| id.trim() == b.pvs_id)
    });
    let vorname = text(&payload.vorname).or_else(|| bekannt.and_then(|b| b.vorname.clone()));
    let nachname = text(&payload.nachname).or_else(|| bekannt.and_then(|b| b.nachname.clone()));
    let geburtsdatum = match optional_datum(payload.geburtsdatum.as_deref(), "geburtsdatum")? {
        Some(d) => Some(d),
        None => bekannt
            .and_then(|b| b.geburtsdatum.as_deref())
            .and_then(datum_lesen),
    };
    let (Some(vorname), Some(nachname), Some(geburtsdatum)) = (vorname, nachname, geburtsdatum)
    else {
        return Err(ThedexFehler::PatientUnbekannt(
            "THEORG braucht Vorname, Nachname und Geburtsdatum; weder Payload noch bekannter Patient liefern alle drei".into(),
        ));
    };

    let zeitpunkt = berichtszeit(payload.datum.as_deref(), jetzt)?;

    let mut n = geruest(Nachrichtentyp::ClientReport, kennung, jetzt)?;
    n.client = Person {
        external_identification: bekannt.and_then(|b| b.externe_id.clone()),
        last_name: Some(nachname),
        first_name: Some(vorname),
        birthday: Some(datum_schreiben(geburtsdatum)),
        ..Default::default()
    };
    n.report_data = Some(Berichtsdaten {
        report_date: Some(datum_schreiben(zeitpunkt.date_naive())),
        report_time: Some(zeit_schreiben(zeitpunkt.time())),
        report_description: Some(payload.titel.trim().to_string()),
        report_therapist: text(&payload.therapeut),
        report_type: Some(typ.to_string()),
        report_file: None,
        weitere: Vec::new(),
    });
    Ok((
        n,
        Anhang {
            inhalt,
            endung: typ.to_string(),
        },
    ))
}

/// `datum` als Datum oder Zeitpunkt lesen, in Berliner Ortszeit.
fn berichtszeit(datum: Option<&str>, jetzt: DateTime<Tz>) -> Ergebnis<DateTime<Tz>> {
    let Some(w) = datum.map(str::trim).filter(|w| !w.is_empty()) else {
        return Ok(jetzt.with_timezone(&Berlin));
    };
    if let Ok(z) = DateTime::parse_from_rfc3339(w) {
        return Ok(z.with_timezone(&Berlin));
    }
    let d = NaiveDate::parse_from_str(w, "%Y-%m-%d")
        .map_err(|_| ThedexFehler::Auftrag(format!("datum {w:?} ist kein ISO-Datum")))?;
    berlin(d, chrono::NaiveTime::MIN)
        .ok_or_else(|| ThedexFehler::Auftrag(format!("datum {w:?} ungültig")))
}

fn optional_datum(w: Option<&str>, feld: &str) -> Ergebnis<Option<NaiveDate>> {
    match w.map(str::trim).filter(|w| !w.is_empty()) {
        None => Ok(None),
        Some(w) => NaiveDate::parse_from_str(w, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| ThedexFehler::Auftrag(format!("{feld} {w:?} ist kein Datum JJJJ-MM-TT"))),
    }
}

fn text(w: &Option<String>) -> Option<String> {
    w.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_types() {
        assert_eq!(report_type("application/pdf", "x"), Some("PDF"));
        assert_eq!(report_type("image/jpeg", "x"), Some("JPG"));
        assert_eq!(report_type("text/plain; charset=utf-8", "x"), Some("TXT"));
        assert_eq!(
            report_type("application/octet-stream", "brief.RTF"),
            Some("RTF")
        );
        assert_eq!(report_type("text/html", ""), Some("HTM"));
        assert_eq!(report_type("image/bmp", ""), Some("BMP"));
        assert_eq!(report_type("application/msword", ""), Some("DOC"));
        assert_eq!(report_type("application/vnd.ms-excel", ""), Some("XLS"));
        assert_eq!(report_type("image/png", "bild.png"), None);
    }

    #[test]
    fn verification() {
        assert!(verification_gueltig("02A7"));
        assert!(!verification_gueltig("02A"));
        assert!(!verification_gueltig("02-7"));
        let mut k = SenderKennung::praxishub("0.1.0");
        assert!(matches!(
            k.pruefen(),
            Err(ThedexFehler::SenderVerificationFehlt)
        ));
        k.verification = Some("1A2B".into());
        assert!(k.pruefen().is_ok());
    }
}
