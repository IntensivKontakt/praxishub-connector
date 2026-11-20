//! Datenmodell einer THEDEX-Nachricht (Spezifikation 2.1, Abschnitt 2.5).
//!
//! Alle Werte bleiben Text, so wie sie in der Datei stehen (`None` = Feld
//! fehlt, `Some("")` = Feld vorhanden, aber leer). Typisierte Auslegung
//! (Datum, Geschlecht, Kostenträger …) passiert erst in der Abbildung
//! ([`crate::v2`]). So geht beim Lesen nichts verloren und die Rundreise
//! Lesen → Schreiben → Lesen ergibt dasselbe Modell.
//!
//! Felder und Abschnitte, die die Spezifikation nicht kennt, landen in
//! `weitere` bzw. [`Nachricht::weitere_abschnitte`] und werden mitgeschrieben.

use crate::fehler::{Ergebnis, ThedexFehler};
use crate::ini::{self, Abschnitt, Feld};
use crate::zeichensatz::Zeichensatz;
use serde::Serialize;

/// Erzeugt einen Abschnittstyp mit festen Feldern (kanonischer Schlüssel plus
/// tolerierte Schreibvarianten) und einer Liste `weitere` für Unbekanntes.
macro_rules! abschnitt_typ {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $( $(#[$fmeta:meta])* $feld:ident : $schl:literal $(| $alias:literal)* ),* $(,)?
        }
        $( zusatz { $( $(#[$xmeta:meta])* $xfeld:ident : $xtyp:ty ),* $(,)? } )?
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
        pub struct $name {
            $(
                $(#[$fmeta])*
                #[serde(skip_serializing_if = "Option::is_none")]
                pub $feld: Option<String>,
            )*
            $($(
                $(#[$xmeta])*
                pub $xfeld: $xtyp,
            )*)?
            /// Felder, die die Spezifikation in diesem Abschnitt nicht kennt.
            #[serde(skip_serializing_if = "Vec::is_empty")]
            pub weitere: Vec<Feld>,
        }

        impl $name {
            /// Kanonische Schlüssel in Schreibreihenfolge.
            pub const SCHLUESSEL: &'static [&'static str] = &[$($schl),*];

            /// Übernimmt ein bekanntes Feld. `false`, wenn der Schlüssel nicht
            /// zu diesem Abschnitt gehört oder schon belegt ist (dann bleibt
            /// das Duplikat in `weitere` erhalten).
            #[allow(unused_variables)]
            fn feld_uebernehmen(&mut self, k: &str, v: &str) -> bool {
                $(
                    if k.eq_ignore_ascii_case($schl) $(|| k.eq_ignore_ascii_case($alias))* {
                        if self.$feld.is_none() {
                            self.$feld = Some(v.to_string());
                            return true;
                        }
                        return false;
                    }
                )*
                false
            }

            fn bekannte_schreiben(&self, out: &mut Vec<Feld>) {
                $(
                    if let Some(w) = &self.$feld {
                        out.push(Feld::neu($schl, w.clone()));
                    }
                )*
            }
        }
    };
}

/// Einfache Abschnitte ohne Sonderfelder.
macro_rules! einfacher_abschnitt {
    ($name:ident) => {
        impl $name {
            pub fn aus_abschnitt(a: &Abschnitt) -> Self {
                let mut s = Self::default();
                for f in &a.felder {
                    if !s.feld_uebernehmen(&f.schluessel, &f.wert) {
                        s.weitere.push(f.clone());
                    }
                }
                s
            }

            pub fn zu_abschnitt(&self, name: &str) -> Abschnitt {
                let mut a = Abschnitt::neu(name);
                self.bekannte_schreiben(&mut a.felder);
                a.felder.extend(self.weitere.iter().cloned());
                a
            }
        }
    };
}

abschnitt_typ! {
    /// `[Thedex]`
    pub struct ThedexInfo {
        version_number: "ThedexVersionNumber",
    }
}
einfacher_abschnitt!(ThedexInfo);

abschnitt_typ! {
    /// `[Sender]` – Kennung des sendenden Programms.
    pub struct Sender {
        identification: "SenderIdentification",
        verification: "SenderVerification",
        description: "SenderDescription",
        software_version_information: "SenderSoftwareVersionInformation",
        /// Im Spec-Beispiel „ClientReport“ als `SenderLanIdetifikation` geschrieben.
        lan_identification: "SenderLanIdentification" | "SenderLanIdetifikation" | "SenderLanIdentifikation",
    }
}
einfacher_abschnitt!(Sender);

abschnitt_typ! {
    /// `[Message]` – Typ, Zeitpunkt und Zeichensatz der Nachricht.
    pub struct Message {
        message_type: "MessageType",
        country_information: "MessageCountryInformation",
        code_page_information: "MessageCodePageInformation",
        date: "MessageDate",
        time: "MessageTime",
    }
}
einfacher_abschnitt!(Message);

abschnitt_typ! {
    /// `[Client]` bzw. `[ClientNewData]` – Personenstammdaten.
    pub struct Person {
        sender_internal_identification: "CSenderInternalIdentification",
        external_identification: "CExternalIdentification",
        title: "CTitle",
        last_name: "CLastName",
        first_name: "CFirstName",
        birthday: "CBirthday",
        sex: "CSex",
        height: "CHeight",
        weight: "CWeight",
        street: "CStreet",
        zip: "CZip",
        town: "CTown",
        country_code: "CCountryCode",
        country: "CCountry",
        /// In den Spec-Beispielen als `CTelephon` geschrieben.
        telephone: "CTelephone" | "CTelephon",
        fax: "CFax",
        check_in_id: "CCheckInID",
        insurance_id: "InsuranceID",
        insurance_client_id: "InsuranceClientID",
        insurance_status: "InsuranceStatus",
        insurance_debitnumber: "InsuranceDebitnumber",
        clinic_department: "ClinicDepartment",
        clinic_room_number: "ClinicRoomNumber",
        clinic_case_number: "ClinicCaseNumber",
        customer_account: "CustomerAccount",
        email: "eMail" | "CEMail" | "CeMail",
        cellular_phone: "CellularPhone" | "CCellularPhone",
        /// So (mit „Idend…“) in der Spezifikation; die korrekte Schreibweise wird toleriert.
        bank_identification_code: "CBankIdendificationCode" | "CBankIdentificationCode",
        bank_name: "CBankName",
        account_number: "CAccountNumber",
        alternative_account_holder: "CAlternativeAccountHolder",
        bic: "CBIC",
        iban: "CIBAN",
    }
    zusatz {
        /// `UserDefined=<n>` und `UserDefined1` … `UserDefined<n>` (Abschnitt 2.6).
        /// `None` = keine UserDefined-Felder in der Datei.
        #[serde(skip_serializing_if = "Option::is_none")]
        user_defined: Option<Vec<String>>,
    }
}

impl Person {
    pub fn aus_abschnitt(a: &Abschnitt) -> Self {
        let mut p = Person::default();
        let mut anzahl: Option<usize> = None;
        let mut eintraege: Vec<(usize, String)> = Vec::new();
        for f in &a.felder {
            if let Some(nr) = user_defined_nummer(&f.schluessel) {
                match nr {
                    None if anzahl.is_none() => {
                        if let Ok(n) = f.wert.trim().parse::<usize>() {
                            // Schutz gegen absurde Angaben; THEORG kennt 6 Zusatzfelder.
                            anzahl = Some(n.min(999));
                            continue;
                        }
                    }
                    Some(i)
                        if (1..=999).contains(&i) && !eintraege.iter().any(|(j, _)| *j == i) =>
                    {
                        eintraege.push((i, f.wert.clone()));
                        continue;
                    }
                    _ => {}
                }
                p.weitere.push(f.clone());
                continue;
            }
            if !p.feld_uebernehmen(&f.schluessel, &f.wert) {
                p.weitere.push(f.clone());
            }
        }
        if anzahl.is_some() || !eintraege.is_empty() {
            let laenge = eintraege
                .iter()
                .map(|(i, _)| *i)
                .max()
                .unwrap_or(0)
                .max(anzahl.unwrap_or(0));
            let mut liste = vec![String::new(); laenge];
            for (i, w) in eintraege {
                liste[i - 1] = w;
            }
            p.user_defined = Some(liste);
        }
        p
    }

    pub fn zu_abschnitt(&self, name: &str) -> Abschnitt {
        let mut a = Abschnitt::neu(name);
        self.bekannte_schreiben(&mut a.felder);
        if let Some(liste) = &self.user_defined {
            a.felder
                .push(Feld::neu("UserDefined", liste.len().to_string()));
            for (i, w) in liste.iter().enumerate() {
                a.felder
                    .push(Feld::neu(format!("UserDefined{}", i + 1), w.clone()));
            }
        }
        a.felder.extend(self.weitere.iter().cloned());
        a
    }
}

/// `UserDefined` → `Some(None)`, `UserDefined7` → `Some(Some(7))`, sonst `None`.
fn user_defined_nummer(schluessel: &str) -> Option<Option<usize>> {
    const PRAEFIX: &str = "userdefined";
    let k = schluessel.to_ascii_lowercase();
    let rest = k.strip_prefix(PRAEFIX)?;
    if rest.is_empty() {
        Some(None)
    } else if rest.bytes().all(|b| b.is_ascii_digit()) {
        Some(rest.parse().ok())
    } else {
        None
    }
}

abschnitt_typ! {
    /// `[ContractData]` – Mitgliedsvertrag (neu in 2.1).
    pub struct Vertragsdaten {
        begin_date: "BeginDate",
        end_date: "EndDate",
        termination_date: "TerminationDate",
        suspend_begin_date: "SuspendBeginDate",
        suspend_end_date: "SuspendEndDate",
        description: "Description",
    }
}
einfacher_abschnitt!(Vertragsdaten);

abschnitt_typ! {
    /// `[ReportData]` – Bericht mit Anhangsdatei.
    pub struct Berichtsdaten {
        report_date: "ReportDate",
        report_time: "ReportTime",
        /// `%personalphoto%` kennzeichnet ein Personenfoto.
        report_description: "ReportDescription",
        report_therapist: "ReportTherapist",
        report_type: "ReportType",
        report_file: "ReportFile",
    }
}
einfacher_abschnitt!(Berichtsdaten);

abschnitt_typ! {
    /// `[CheckInOutData]` – Check-in/-out (neu in 2.1).
    pub struct CheckInOutDaten {
        event: "Event",
        check_in_date: "CheckInDate",
        check_in_time: "CheckInTime",
        check_out_date: "CheckOutDate",
        check_out_time: "CheckOutTime",
        description: "Description",
    }
}
einfacher_abschnitt!(CheckInOutDaten);

/// Nachrichtentyp aus `MessageType`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Nachrichtentyp {
    ClientNew,
    ClientChange,
    ClientInfo,
    ClientDelete,
    ClientReport,
    ClientCheckInOut,
    Unbekannt(String),
}

impl Nachrichtentyp {
    pub fn aus_text(wert: &str) -> Self {
        let w = wert.trim();
        for t in [
            Nachrichtentyp::ClientNew,
            Nachrichtentyp::ClientChange,
            Nachrichtentyp::ClientInfo,
            Nachrichtentyp::ClientDelete,
            Nachrichtentyp::ClientReport,
            Nachrichtentyp::ClientCheckInOut,
        ] {
            if w.eq_ignore_ascii_case(t.als_text()) {
                return t;
            }
        }
        Nachrichtentyp::Unbekannt(w.to_string())
    }

    pub fn als_text(&self) -> &str {
        match self {
            Nachrichtentyp::ClientNew => "ClientNew",
            Nachrichtentyp::ClientChange => "ClientChange",
            Nachrichtentyp::ClientInfo => "ClientInfo",
            Nachrichtentyp::ClientDelete => "ClientDelete",
            Nachrichtentyp::ClientReport => "ClientReport",
            Nachrichtentyp::ClientCheckInOut => "ClientCheckInOut",
            Nachrichtentyp::Unbekannt(s) => s,
        }
    }
}

/// Eine THEDEX-Nachricht (= eine `.TDX`-Datei).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Nachricht {
    /// `[Thedex]` – Pflichtabschnitt, beim Lesen toleriert, wenn er fehlt.
    pub thedex: Option<ThedexInfo>,
    /// `[Sender]` – Pflichtabschnitt, beim Lesen toleriert, wenn er fehlt.
    pub sender: Option<Sender>,
    /// `[Message]` – ohne ihn ist die Datei nicht verwertbar.
    pub message: Message,
    /// `[Client]` – ohne ihn ist die Datei nicht verwertbar.
    pub client: Person,
    /// `[ClientNewData]` – nur bei `ClientChange`: die neuen Stammdaten.
    pub client_new_data: Option<Person>,
    /// `[ContractData]` – jeder Abschnitt dieses Namens ein Eintrag (die
    /// Spezifikation sagt nicht, ob mehrere Verträge vorkommen).
    pub contract_data: Vec<Vertragsdaten>,
    pub report_data: Option<Berichtsdaten>,
    pub check_in_out_data: Option<CheckInOutDaten>,
    /// Unbekannte oder doppelte Abschnitte (z. B. `[Cybex]`), unverändert.
    pub weitere_abschnitte: Vec<Abschnitt>,
}

impl Nachricht {
    /// Leere Nachricht eines Typs mit `[Thedex]`-Version 2.1 und ANSI.
    pub fn neu(typ: Nachrichtentyp) -> Self {
        Nachricht {
            thedex: Some(ThedexInfo {
                version_number: Some(crate::THEDEX_VERSION.to_string()),
                ..Default::default()
            }),
            sender: None,
            message: Message {
                message_type: Some(typ.als_text().to_string()),
                country_information: Some("049".to_string()),
                code_page_information: Some(Zeichensatz::Ansi.kennung().to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn typ(&self) -> Nachrichtentyp {
        Nachrichtentyp::aus_text(self.message.message_type.as_deref().unwrap_or(""))
    }

    pub fn zeichensatz(&self) -> Zeichensatz {
        Zeichensatz::aus_kennung(self.message.code_page_information.as_deref().unwrap_or(""))
    }

    /// Die maßgeblichen Stammdaten: bei `ClientChange` `[ClientNewData]`
    /// (falls vorhanden), sonst `[Client]`.
    pub fn aktuelle_person(&self) -> &Person {
        match (&self.typ(), &self.client_new_data) {
            (Nachrichtentyp::ClientChange, Some(neu)) => neu,
            _ => &self.client,
        }
    }

    /// Fehlende Pflichtabschnitte, die beim Lesen toleriert wurden.
    pub fn maengel(&self) -> Vec<String> {
        let mut m = Vec::new();
        if self.thedex.is_none() {
            m.push("Pflichtabschnitt [Thedex] fehlt".to_string());
        }
        if self.sender.is_none() {
            m.push("Pflichtabschnitt [Sender] fehlt".to_string());
        }
        if self.typ() == Nachrichtentyp::ClientChange && self.client_new_data.is_none() {
            m.push("ClientChange ohne [ClientNewData]: [Client] wird verwendet".to_string());
        }
        m
    }

    // ---------------------------------------------------------------- Lesen

    /// Liest eine Nachricht aus Dateibytes. Zeichensatz laut
    /// `MessageCodePageInformation` (fehlt er: ANSI). Eine Datei mit UTF-8-BOM
    /// wird als UTF-8 gelesen.
    pub fn aus_bytes(bytes: &[u8]) -> Ergebnis<Self> {
        if bytes.contains(&0) {
            return Err(ThedexFehler::Format(
                "Datei enthält Binärdaten (NUL-Bytes)".into(),
            ));
        }
        if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
            return Self::aus_text(&String::from_utf8_lossy(rest));
        }
        // Schlüssel sind ASCII; erst als ANSI lesen, dann ggf. als OEM neu.
        let n = Self::aus_text(&Zeichensatz::Ansi.dekodieren(bytes))?;
        match n.zeichensatz() {
            Zeichensatz::Ansi => Ok(n),
            Zeichensatz::Oem => Self::aus_text(&Zeichensatz::Oem.dekodieren(bytes)),
        }
    }

    /// Liest eine Nachricht aus bereits dekodiertem Text.
    pub fn aus_text(text: &str) -> Ergebnis<Self> {
        let zerlegt = ini::zerlegen(text);
        let mut n = Nachricht::default();
        let mut message: Option<Message> = None;
        let mut client: Option<Person> = None;

        for a in zerlegt.abschnitte {
            let name = a.name.to_ascii_lowercase();
            match name.as_str() {
                "thedex" if n.thedex.is_none() => n.thedex = Some(ThedexInfo::aus_abschnitt(&a)),
                "sender" if n.sender.is_none() => n.sender = Some(Sender::aus_abschnitt(&a)),
                "message" if message.is_none() => message = Some(Message::aus_abschnitt(&a)),
                "client" if client.is_none() => client = Some(Person::aus_abschnitt(&a)),
                "clientnewdata" if n.client_new_data.is_none() => {
                    n.client_new_data = Some(Person::aus_abschnitt(&a))
                }
                "contractdata" => n.contract_data.push(Vertragsdaten::aus_abschnitt(&a)),
                "reportdata" if n.report_data.is_none() => {
                    n.report_data = Some(Berichtsdaten::aus_abschnitt(&a))
                }
                "checkinoutdata" if n.check_in_out_data.is_none() => {
                    n.check_in_out_data = Some(CheckInOutDaten::aus_abschnitt(&a))
                }
                _ => n.weitere_abschnitte.push(a),
            }
        }

        n.message = message
            .ok_or_else(|| ThedexFehler::Format("Pflichtabschnitt [Message] fehlt".into()))?;
        if n.message
            .message_type
            .as_deref()
            .is_none_or(|t| t.trim().is_empty())
        {
            return Err(ThedexFehler::Format("MessageType fehlt".into()));
        }
        n.client =
            client.ok_or_else(|| ThedexFehler::Format("Pflichtabschnitt [Client] fehlt".into()))?;
        Ok(n)
    }

    // -------------------------------------------------------------- Schreiben

    /// Abschnitte in Schreibreihenfolge.
    pub fn abschnitte(&self) -> Vec<Abschnitt> {
        let mut v = Vec::new();
        if let Some(t) = &self.thedex {
            v.push(t.zu_abschnitt("Thedex"));
        }
        if let Some(s) = &self.sender {
            v.push(s.zu_abschnitt("Sender"));
        }
        v.push(self.message.zu_abschnitt("Message"));
        v.push(self.client.zu_abschnitt("Client"));
        if let Some(p) = &self.client_new_data {
            v.push(p.zu_abschnitt("ClientNewData"));
        }
        for c in &self.contract_data {
            v.push(c.zu_abschnitt("ContractData"));
        }
        if let Some(r) = &self.report_data {
            v.push(r.zu_abschnitt("ReportData"));
        }
        if let Some(c) = &self.check_in_out_data {
            v.push(c.zu_abschnitt("CheckInOutData"));
        }
        v.extend(self.weitere_abschnitte.iter().cloned());
        v
    }

    /// Text der Datei (Windows-Zeilenenden).
    pub fn als_text(&self) -> String {
        ini::schreiben(&self.abschnitte())
    }

    /// Dateibytes im Zeichensatz laut `MessageCodePageInformation`.
    pub fn als_bytes(&self) -> Vec<u8> {
        self.zeichensatz().kodieren(&self.als_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "[Thedex]\r\nThedexVersionNumber=2.1\r\n\r\n[Message]\r\nMessageType=ClientInfo\r\n\r\n[Client]\r\nCLastName=Muster\r\n";

    #[test]
    fn minimal_lesen_und_schreiben() {
        let n = Nachricht::aus_text(MINIMAL).unwrap();
        assert_eq!(n.typ(), Nachrichtentyp::ClientInfo);
        assert_eq!(n.client.last_name.as_deref(), Some("Muster"));
        assert_eq!(n.als_text(), MINIMAL);
        assert_eq!(
            n.maengel(),
            vec!["Pflichtabschnitt [Sender] fehlt".to_string()]
        );
    }

    #[test]
    fn user_defined_luecken_und_anzahl() {
        let n = Nachricht::aus_text(
            "[Message]\nMessageType=ClientInfo\n[Client]\nUserDefined=4\nUserDefined2=b\nuserdefined1=a\nUserDefinedX=y\n",
        )
        .unwrap();
        assert_eq!(
            n.client.user_defined,
            Some(vec!["a".into(), "b".into(), "".into(), "".into()])
        );
        assert_eq!(n.client.weitere, vec![Feld::neu("UserDefinedX", "y")]);
    }

    #[test]
    fn doppelte_felder_und_abschnitte_bleiben_erhalten() {
        let text = "[Message]\nMessageType=ClientInfo\n[Client]\nCLastName=A\nCLastName=B\n[Client]\nCLastName=C\n";
        let n = Nachricht::aus_text(text).unwrap();
        assert_eq!(n.client.last_name.as_deref(), Some("A"));
        assert_eq!(n.client.weitere, vec![Feld::neu("CLastName", "B")]);
        assert_eq!(n.weitere_abschnitte.len(), 1);
        assert_eq!(Nachricht::aus_text(&n.als_text()).unwrap(), n);
    }

    #[test]
    fn typ_unbekannt() {
        assert_eq!(
            Nachrichtentyp::aus_text("clientcheckinout"),
            Nachrichtentyp::ClientCheckInOut
        );
        assert_eq!(
            Nachrichtentyp::aus_text("Foo"),
            Nachrichtentyp::Unbekannt("Foo".into())
        );
    }
}
