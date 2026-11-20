//! Abbildung von THEDEX-Nachrichten auf Objekte des Connector-v2-Vertrags
//! (Walletpass `docs/connector-v2-vertrag.md`, Abschnitt 5).
//!
//! * `[Client]`/`[ClientNewData]` → [`PatientV2`] (Entität `patienten`)
//! * `[ContractData]`             → [`MitgliedschaftV2`] (neue Entität `mitgliedschaften`)
//! * `[CheckInOutData]`           → [`CheckinV2`] (neue Entität `checkins`)
//!
//! `pvs_id` eines Patienten ist die THEORG-Personennummer
//! (`CSenderInternalIdentification`). Die gilt nur, wenn THEORG selbst der
//! Sender ist; Nachrichten anderer Provider (Geräte, Cybex …) tragen dort ihre
//! eigene Nummer und werden deshalb nicht abgebildet, nur gemeldet.

use crate::modell::{CheckInOutDaten, Nachricht, Nachrichtentyp, Person, Vertragsdaten};
use crate::werte::{datum_lesen, gefuellt, zeit_lesen};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::{Europe::Berlin, Tz};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Maximale Länge einer `pvs_id` laut Vertrag.
pub const PVS_ID_MAX: usize = 128;

/// `PatientV2` laut Vertrag plus `checkin_id` und `externe_id` (unbekannte
/// Felder spiegelt die Cloud, ohne sie auszuwerten).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatientV2 {
    pub pvs_id: String,
    pub geloescht: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geaendert_am: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vorname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nachname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geburtsdatum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geschlecht: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telefon_mobil: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telefon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strasse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plz: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kostentraeger: Option<String>,
    /// `CCheckInID` – Nummer des Check-in-Mediums (Chip/Karte).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkin_id: Option<String>,
    /// `CExternalIdentification` – bei von Praxishub angelegten Patienten die
    /// Praxishub-Kennung (siehe [`crate::ausgang`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub externe_id: Option<String>,
}

/// Mitgliedsvertrag (`[ContractData]`), neue Entität `mitgliedschaften`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MitgliedschaftV2 {
    pub pvs_id: String,
    pub geloescht: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geaendert_am: Option<String>,
    pub patient_pvs_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bezeichnung: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beginn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ende: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kuendigung_zum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ruhezeit_von: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ruhezeit_bis: Option<String>,
}

/// Check-in/-out (`[CheckInOutData]`), neue Entität `checkins`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckinV2 {
    pub pvs_id: String,
    pub geloescht: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geaendert_am: Option<String>,
    pub patient_pvs_id: String,
    /// `in` | `out`
    pub ereignis: String,
    /// ISO 8601 mit Offset (Europe/Berlin).
    pub zeitpunkt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notiz: Option<String>,
}

/// Ergebnis der Abbildung einer Nachricht.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Abbildung {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub patienten: Vec<PatientV2>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mitgliedschaften: Vec<MitgliedschaftV2>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checkins: Vec<CheckinV2>,
    /// Was nicht abgebildet werden konnte und warum (fürs Protokoll).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hinweise: Vec<String>,
}

impl Abbildung {
    pub fn ist_leer(&self) -> bool {
        self.patienten.is_empty() && self.mitgliedschaften.is_empty() && self.checkins.is_empty()
    }
}

/// Einstellungen der Abbildung.
#[derive(Debug, Clone)]
pub struct AbbildungsOptionen {
    /// Provider, dessen Personennummern als `pvs_id` gelten (Standard `THEORG`).
    pub pvs_provider: String,
}

impl Default for AbbildungsOptionen {
    fn default() -> Self {
        AbbildungsOptionen {
            pvs_provider: "THEORG".to_string(),
        }
    }
}

/// Bildet eine Nachricht auf v2-Objekte ab.
pub fn abbilden(n: &Nachricht, opt: &AbbildungsOptionen) -> Abbildung {
    let mut ab = Abbildung::default();
    for m in n.maengel() {
        ab.hinweise.push(m);
    }

    let sender = n
        .sender
        .as_ref()
        .and_then(|s| gefuellt(&s.identification))
        .unwrap_or("");
    if !sender.eq_ignore_ascii_case(&opt.pvs_provider) {
        ab.hinweise.push(format!(
            "Sender {sender:?} ist nicht {:?}: Personennummern gelten nicht als pvs_id, nichts abgebildet",
            opt.pvs_provider
        ));
        return ab;
    }

    let typ = n.typ();
    if let Nachrichtentyp::Unbekannt(t) = &typ {
        ab.hinweise
            .push(format!("unbekannter MessageType {t:?}, nichts abgebildet"));
        return ab;
    }

    let person = n.aktuelle_person();
    let Some(pvs_id) = gefuellt(&person.sender_internal_identification)
        .or_else(|| gefuellt(&n.client.sender_internal_identification))
        .map(str::to_string)
    else {
        ab.hinweise.push(
            "CSenderInternalIdentification fehlt: Person ohne THEORG-Nummer nicht abbildbar".into(),
        );
        return ab;
    };
    if pvs_id.chars().count() > PVS_ID_MAX {
        ab.hinweise
            .push(format!("Personennummer länger als {PVS_ID_MAX} Zeichen"));
        return ab;
    }

    let geloescht = typ == Nachrichtentyp::ClientDelete;
    let geaendert_am = nachrichtenzeit(n).map(|z| z.to_rfc3339());

    let mut patient = patient_aus_person(&pvs_id, person);
    patient.geloescht = geloescht;
    patient.geaendert_am = geaendert_am.clone();
    if !geloescht && patient.vorname.is_none() && patient.nachname.is_none() {
        ab.hinweise
            .push("weder CFirstName noch CLastName: Patient nicht abgebildet".into());
    } else {
        ab.patienten.push(patient);
    }

    for v in &n.contract_data {
        match mitgliedschaft(&pvs_id, v) {
            Some(mut m) => {
                m.geloescht = geloescht;
                m.geaendert_am = geaendert_am.clone();
                ab.mitgliedschaften.push(m);
            }
            None => ab
                .hinweise
                .push("[ContractData] ohne Inhalt übersprungen".into()),
        }
    }

    if let Some(c) = &n.check_in_out_data {
        match checkin(&pvs_id, c, nachrichtenzeit(n)) {
            Ok(mut ci) => {
                ci.geaendert_am = geaendert_am.clone();
                ab.checkins.push(ci);
            }
            Err(h) => ab.hinweise.push(h),
        }
    } else if typ == Nachrichtentyp::ClientCheckInOut {
        ab.hinweise
            .push("ClientCheckInOut ohne [CheckInOutData]".into());
    }

    ab
}

/// Stammdaten einer Person → `PatientV2` (ohne `geloescht`/`geaendert_am`).
pub fn patient_aus_person(pvs_id: &str, p: &Person) -> PatientV2 {
    let text = |w: &Option<String>| gefuellt(w).map(str::to_string);
    PatientV2 {
        pvs_id: pvs_id.to_string(),
        vorname: text(&p.first_name),
        nachname: text(&p.last_name),
        geburtsdatum: gefuellt(&p.birthday).and_then(datum_lesen).map(iso_datum),
        geschlecht: gefuellt(&p.sex).and_then(geschlecht),
        telefon_mobil: text(&p.cellular_phone),
        telefon: text(&p.telephone),
        email: text(&p.email),
        strasse: text(&p.street),
        plz: text(&p.zip),
        ort: text(&p.town),
        kostentraeger: kostentraeger(p),
        checkin_id: text(&p.check_in_id),
        externe_id: text(&p.external_identification),
        ..Default::default()
    }
}

/// `CSex`: `W` (in den Spec-Beispielen auch `F`) → `w`, `M` → `m`, `D`/`X` → `d`.
pub fn geschlecht(wert: &str) -> Option<String> {
    match wert.trim().to_ascii_uppercase().as_str() {
        "W" | "F" => Some("w".into()),
        "M" => Some("m".into()),
        "D" | "X" => Some("d".into()),
        _ => None,
    }
}

/// Heuristik aus `InsuranceStatus`/`InsuranceID`:
/// Status `P` → `selbstzahler`; neunstellige IK-Nummer → `gkv`; sonst nichts.
pub fn kostentraeger(p: &Person) -> Option<String> {
    let status = gefuellt(&p.insurance_status).unwrap_or("");
    if status.to_ascii_uppercase().starts_with('P') {
        return Some("selbstzahler".into());
    }
    let ik = gefuellt(&p.insurance_id).unwrap_or("");
    if ik.len() == 9 && ik.bytes().all(|b| b.is_ascii_digit()) {
        return Some("gkv".into());
    }
    None
}

/// `[ContractData]` → Mitgliedschaft. `None`, wenn alle Felder leer sind.
pub fn mitgliedschaft(patient_pvs_id: &str, v: &Vertragsdaten) -> Option<MitgliedschaftV2> {
    let iso = |w: &Option<String>| gefuellt(w).and_then(datum_lesen).map(iso_datum);
    let m = MitgliedschaftV2 {
        pvs_id: String::new(),
        patient_pvs_id: patient_pvs_id.to_string(),
        bezeichnung: gefuellt(&v.description).map(str::to_string),
        beginn: iso(&v.begin_date),
        ende: iso(&v.end_date),
        kuendigung_zum: iso(&v.termination_date),
        ruhezeit_von: iso(&v.suspend_begin_date),
        ruhezeit_bis: iso(&v.suspend_end_date),
        ..Default::default()
    };
    if m.bezeichnung.is_none()
        && m.beginn.is_none()
        && m.ende.is_none()
        && m.kuendigung_zum.is_none()
        && m.ruhezeit_von.is_none()
        && m.ruhezeit_bis.is_none()
    {
        return None;
    }
    Some(MitgliedschaftV2 {
        pvs_id: mitgliedschaft_id(
            patient_pvs_id,
            m.beginn.as_deref(),
            m.bezeichnung.as_deref(),
        ),
        ..m
    })
}

/// Stabile Kennung eines Vertrags: `<Patient>:<Beginn JJJJMMTT|->:<16 Hex aus
/// SHA-256 der Bezeichnung>`. THEDEX liefert keine Vertragsnummer; ändert sich
/// Beginn oder Bezeichnung in THEORG, entsteht eine neue Kennung.
pub fn mitgliedschaft_id(
    patient: &str,
    beginn_iso: Option<&str>,
    bezeichnung: Option<&str>,
) -> String {
    let beginn = beginn_iso
        .map(|b| b.replace('-', ""))
        .unwrap_or_else(|| "-".into());
    let bez = bezeichnung.unwrap_or("").trim().to_lowercase();
    let id = format!("{patient}:{beginn}:{}", &hex_sha256(&bez)[..16]);
    begrenzen(id)
}

/// `[CheckInOutData]` → Check-in. Zeitpunkt: bei `IN` CheckIn-Datum/-Zeit,
/// bei `OUT` CheckOut-Datum/-Zeit; fehlt die Zeit, gilt der Nachrichtenzeitpunkt.
pub fn checkin(
    patient_pvs_id: &str,
    c: &CheckInOutDaten,
    nachrichtenzeit: Option<DateTime<Tz>>,
) -> Result<CheckinV2, String> {
    let ereignis = match gefuellt(&c.event).map(str::to_ascii_uppercase).as_deref() {
        Some("IN") => "in",
        Some("OUT") => "out",
        other => return Err(format!("[CheckInOutData] Event {other:?} unbekannt")),
    };
    let (datum, zeit) = if ereignis == "in" {
        (&c.check_in_date, &c.check_in_time)
    } else {
        (&c.check_out_date, &c.check_out_time)
    };
    let zeitpunkt = match (
        gefuellt(datum).and_then(datum_lesen),
        gefuellt(zeit).and_then(zeit_lesen),
    ) {
        (Some(d), Some(t)) => berlin(d, t),
        _ => nachrichtenzeit,
    }
    .ok_or_else(|| "[CheckInOutData] ohne verwertbaren Zeitpunkt".to_string())?;
    Ok(CheckinV2 {
        pvs_id: begrenzen(format!(
            "{patient_pvs_id}:{ereignis}:{}",
            zeitpunkt.format("%Y%m%d%H%M")
        )),
        geloescht: false,
        geaendert_am: None,
        patient_pvs_id: patient_pvs_id.to_string(),
        ereignis: ereignis.to_string(),
        zeitpunkt: zeitpunkt.to_rfc3339(),
        notiz: gefuellt(&c.description).map(str::to_string),
    })
}

/// Zeitpunkt aus `MessageDate`/`MessageTime` (Europe/Berlin).
pub fn nachrichtenzeit(n: &Nachricht) -> Option<DateTime<Tz>> {
    let d = gefuellt(&n.message.date).and_then(datum_lesen)?;
    let t = gefuellt(&n.message.time)
        .and_then(zeit_lesen)
        .unwrap_or(NaiveTime::MIN);
    berlin(d, t)
}

/// Ortszeit Berlin → Zeitpunkt mit Offset. In der Zeitumstellung gilt bei
/// doppelter Stunde die frühere, eine nicht existierende Stunde wird um eine
/// Stunde nach vorn geschoben.
pub fn berlin(d: NaiveDate, t: NaiveTime) -> Option<DateTime<Tz>> {
    let lokal = NaiveDateTime::new(d, t);
    Berlin.from_local_datetime(&lokal).earliest().or_else(|| {
        Berlin
            .from_local_datetime(&(lokal + chrono::Duration::hours(1)))
            .earliest()
    })
}

fn iso_datum(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn hex_sha256(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Kennungen über 128 Zeichen werden durch ihren SHA-256 (64 Hex) ersetzt.
fn begrenzen(id: String) -> String {
    if id.chars().count() <= PVS_ID_MAX {
        id
    } else {
        hex_sha256(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geschlecht_tolerant() {
        assert_eq!(geschlecht("F").as_deref(), Some("w"));
        assert_eq!(geschlecht("w").as_deref(), Some("w"));
        assert_eq!(geschlecht("M").as_deref(), Some("m"));
        assert_eq!(geschlecht(""), None);
        assert_eq!(geschlecht("?"), None);
    }

    #[test]
    fn kostentraeger_heuristik() {
        let mut p = Person {
            insurance_status: Some("P".into()),
            insurance_id: Some("108018347".into()),
            ..Default::default()
        };
        assert_eq!(kostentraeger(&p).as_deref(), Some("selbstzahler"));
        p.insurance_status = Some("10001".into());
        assert_eq!(kostentraeger(&p).as_deref(), Some("gkv"));
        p.insurance_id = Some("PRIVAT".into());
        assert_eq!(kostentraeger(&p), None);
        p.insurance_id = Some(String::new());
        p.insurance_status = Some(String::new());
        assert_eq!(kostentraeger(&p), None);
    }

    #[test]
    fn mitgliedschaft_id_stabil_und_begrenzt() {
        let a = mitgliedschaft_id("1166", Some("2014-01-01"), Some("Gerätetraining"));
        let b = mitgliedschaft_id("1166", Some("2014-01-01"), Some(" gerätetraining "));
        assert_eq!(a, b);
        assert!(a.starts_with("1166:20140101:"));
        let lang = mitgliedschaft_id(&"9".repeat(200), None, None);
        assert_eq!(lang.len(), 64);
    }

    #[test]
    fn berlin_offsets() {
        let d = NaiveDate::from_ymd_opt(2014, 9, 11).unwrap();
        let t = NaiveTime::from_hms_opt(17, 17, 0).unwrap();
        assert_eq!(
            berlin(d, t).unwrap().to_rfc3339(),
            "2014-09-11T17:17:00+02:00"
        );
        let w = NaiveDate::from_ymd_opt(2014, 1, 11).unwrap();
        assert_eq!(
            berlin(w, t).unwrap().to_rfc3339(),
            "2014-01-11T17:17:00+01:00"
        );
        // Lücke bei der Umstellung auf Sommerzeit (30.03.2014, 02:30 gibt es nicht)
        let luecke = NaiveDate::from_ymd_opt(2014, 3, 30).unwrap();
        let t = NaiveTime::from_hms_opt(2, 30, 0).unwrap();
        assert_eq!(
            berlin(luecke, t).unwrap().to_rfc3339(),
            "2014-03-30T03:30:00+02:00"
        );
    }
}
