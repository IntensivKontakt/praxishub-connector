//! HTTPS-Anbindung an die Praxishub-Cloud.
//!
//! Auth wie bei der Doctolib-Extension: `Authorization: Bearer <api_key>` +
//! `X-Praxishub-Tenant`. Endpunkte unter `/api/v1/connector/*`.
//!
//! **Backend-Arbeit offen:** diese Routen müssen in der Praxishub-API noch
//! angelegt werden (heartbeat / hkp). Vertrag siehe [`HkpReport`].

use crate::config::ConnectorConfig;
use crate::error::{ConnectorError, Result};
use crate::z1db::WritebackReport;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Body des `applied`-Acks inkl. optionalem Schreib-Report — die Cloud macht damit
/// sichtbar, ob z. B. die Risikoanamnese (CAVE) wirklich geschrieben oder
/// übersprungen wurde. Alle Report-Felder werden weggelassen, wenn kein Report vorliegt.
#[derive(Debug, Serialize)]
struct WritebackAppliedBody<'a> {
    patient_id: &'a str,
    matched_by: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    contact_updated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    address_updated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cave_appended: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    co_appended: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    anamnese_inserted: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes_inserted: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    skipped: Vec<String>,
}

#[derive(Clone)]
pub struct CloudClient {
    http: reqwest::Client,
    base_url: String,
    tenant_id: String,
    api_key: String,
}

/// Eine erkannte, genehmigte HKP-/EBZ-Nachricht. Der Connector parst NICHT — er
/// liefert die (bereits entschlüsselte) Rohnachricht; die Cloud macht das
/// autoritative CMS/.p7s/XML-Parsing.
#[derive(Debug, Serialize)]
pub struct HkpReport {
    /// Stabiler Dedup-Schlüssel = POP3-UIDL.
    pub source_uidl: String,
    pub dienstkennung: String,
    pub message_id: Option<String>,
    /// Empfangszeitpunkt laut Mail-Header.
    pub received_at: Option<String>,
    /// Komplette RFC822-Nachricht (Base64), bereits vom KIM-Clientmodul entschlüsselt.
    pub raw_message_b64: String,
}

/// Ein vom Backend zur Ablage in die PVS-Akte bereitgestelltes Dokument
/// (unterschriebene Anamnese / HKP-PDF). Die Z1-`PATID` liegt laut Backend in
/// ~90 % der Fälle bereits vor; sonst greift der Name/Geburtsdatum-Fallback.
///
/// **Backend-Vertrag offen:** Route `GET /api/v1/connector/documents/pending`
/// muss in der Praxishub-API noch angelegt werden (analog zu `hkp`).
#[derive(Debug, Clone, Deserialize)]
pub struct PendingDocument {
    /// Backend-Dokument-ID (Idempotenz-/Ack-Schlüssel).
    pub id: String,
    /// `"anamnese"` | `"hkp"` | `"rechnung"` | `"storno"` | `"anamnese_upload"`.
    #[serde(default)]
    pub kind: String,
    /// MIME-Typ des Objekts. Leer ⇒ PDF (rückwärtskompatibel für generierte Belege).
    /// Bei `kind="anamnese_upload"` genau einer von `application/pdf` |
    /// `image/jpeg` | `image/png` (HEIC/WEBP wandelt die Cloud vorab nach JPEG).
    #[serde(default)]
    pub content_type: String,
    /// Original-Dateiname (z. B. `"roentgen.jpg"`); leer bei generierten PDFs.
    #[serde(default)]
    pub filename: String,
    /// Z1-interne PATID, falls dem Backend bekannt (sonst leer → Fallback).
    #[serde(default)]
    pub patient_id: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub first_name: String,
    /// Geburtsdatum. Das Backend liefert es im Z1-Format `JJJJMMTT`; die
    /// Zuordnung normalisiert ohnehin formatunabhängig (siehe [`crate::matching`]).
    #[serde(default)]
    pub birth_date: String,
    /// Postleitzahl aus dem Anamnese-Formular — Tiebreaker bei Namensvettern.
    #[serde(default)]
    pub zip: String,
    /// E-Mail aus dem Anamnese-Formular — Tiebreaker bei Namensvettern.
    #[serde(default)]
    pub email: String,
    /// Das abzulegende PDF, Base64-kodiert.
    pub pdf_base64: String,
}

/// Ein von der Cloud geliefertes Rückschreib-Bündel (digitale Aufnahme → Z1).
/// Analog zu [`PendingDocument`], aber für **strukturierte** Felder statt PDF.
///
/// **Backend-Vertrag offen:** `GET /api/v1/connector/z1/writeback/pending`.
#[derive(Debug, Clone, Deserialize)]
pub struct PendingWriteback {
    /// Backend-ID (Idempotenz-/Ack-Schlüssel).
    pub id: String,
    /// Z1-`PATNR`, falls dem Backend bekannt (sonst leer → Name/Geburtsdatum-Lookup).
    #[serde(default)]
    pub patient_id: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub first_name: String,
    /// Geburtsdatum (Format egal — wird beim Lookup normalisiert).
    #[serde(default)]
    pub birth_date: String,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    /// Straße inkl. Hausnummer.
    #[serde(default)]
    pub street: Option<String>,
    /// Adresszusatz (z. B. „c/o …", Wohnung) → ADR.ANSCHRIFTENZUSATZ.
    #[serde(default)]
    pub address_addendum: Option<String>,
    #[serde(default)]
    pub zip: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    /// CAVE-/Allergie-Einträge (additiv an die Risikoanamnese).
    #[serde(default)]
    pub cave: Vec<String>,
    /// Krankenanamnese-Zeilen (→ PATINFO ART=1). **Nur klinische Anamnese** —
    /// KEINE Rechnungsstatus/Verwaltungsnotizen (dafür `notes`).
    #[serde(default)]
    pub anamnese: Vec<String>,
    /// Karteikarten-/Verlaufsnotizen (z. B. „Rechnung AH-2026-0012 über 85,00 €
    /// bezahlt") → `BEH`-Freitext (GOART leer). Eigener Kanal, getrennt von
    /// `anamnese`, damit Verwaltungsnotizen nicht in der Krankenanamnese landen.
    ///
    /// **Backend-Vertrag (Walletpass-Repo, `connector_writeback.py` /
    /// `crm_invoice_connector.py`):** Rechnungsstatus-Notizen ab jetzt in `notes[]`
    /// statt `anamnese[]` liefern. `notes` ist additiv und `#[serde(default)]` —
    /// eine alte Cloud, die weiter `anamnese[]` schickt, bricht NICHT (der
    /// Connector legt das dann weiterhin in PATINFO ab, altes Verhalten). Der
    /// Connector schreibt `notes[]` nur, wenn der Toggle `writeback_notes` aktiv
    /// ist, und quittiert die Zahl im `applied`-Ack als `notes_inserted`.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Ein **HKP-Fall** (`PATNR`+`LFDBEFUND`) fürs Praxishub-Tracking-Modul: eine
/// Kachel pro Fall, Status vom führenden (GAV-)Plan, plus alle Pläne des Falls
/// (GAV-Kassenplan + AAV-Privatalternative) mit ihrem EBZ-Verlauf fürs Drawer.
/// Ersetzt den KIM-Weg ([`HkpReport`]) durch den DB-Weg.
///
/// Wird gemeldet, sobald sich der `status` des Falls ändert. Cloud upsertet je
/// `case_key`. `status` ∈ { `erstellt`, `versendet`, `rueckfrage`, `genehmigt`,
/// `abgelehnt`, `abgelaufen`, `eingegliedert`, `abgerechnet` } (`signiert`→`erstellt`).
///
/// **`abgelaufen`** = genehmigt, nicht eingegliedert, in Z1 deaktiviert
/// (`DEAKTIVIERTDATUM`) **oder** über Gültigkeit (Genehmigung+6M) → verlorener Umsatz.
/// `valid_until` → „Tage bis Ablauf"; „genehmigt & nicht terminiert" bildet Praxishub
/// aus `status`=genehmigt + eigener Terminplanung (Termine nicht in Z1).
///
/// **Backend-Vertrag offen:** `POST /api/v1/connector/z1/hkp-status`.
#[derive(Debug, Clone, Serialize)]
pub struct HkpCaseReport {
    /// Stabiler Fall-Schlüssel (`PATNR|LFDBEFUND`).
    pub case_key: String,
    pub patient_id: String,
    /// Befund-/Fallnummer (`LFDBEFUND`).
    pub befund_no: String,
    /// Dekodierte Planart des Falls (`eHKP`, `ePAR`, `eKBR/KGL`).
    pub planart: String,
    /// Aktueller Fall-Status = Status des führenden GAV-Plans.
    pub status: String,
    // Meilenstein-Daten des führenden Plans (`JJJJMMTT`), soweit erreicht.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inserted_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billed_on: Option<String>,
    /// Gültigkeitsende (Genehmigung + 6 Monate) des führenden Plans.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    /// Behandler-Anzeigename des führenden Plans (`ZPLAN.LEBID` → `PERSONAL`,
    /// Gematik-Vor-/Nachname; Fallback: Kürzel).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behandler: Option<String>,
    /// Behandler-Kürzel (`PERSONAL.KUERZEL`, z. B. `"st"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behandler_kuerzel: Option<String>,
    /// Gesamtbetrag des Plans in Euro. ZE: `zer:Behandlungskosten_insgesamt` aus
    /// dem EEBZ0-XML. ePAR: Schätzung geplante BEMA-Punkte × `z1_par_punktwert`
    /// (der ePAR-Antrag enthält keine Euro-Beträge). Nur gesetzt, wenn ableitbar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub betrag_gesamt: Option<f64>,
    /// Patientenanteil in Euro. Steht NICHT im Antrag (EEBZ0) — bleibt vorerst
    /// leer; berechenbar erst aus der Kassen-Antwort (EEBZ1, Festzuschüsse).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub betrag_patientenanteil: Option<f64>,
    /// Echte Leistungsbeschreibung, gekappt — ZE: dedupliziert aus
    /// `zer:Leistungsbeschreibung`; ePAR: Klassifikation + geplante Positionen
    /// (z. B. `PAR-Therapie Stadium III Grad B: …, AIT a ×8, … (417 BEMA-Punkte)`);
    /// eKBR/KGL: Freitext + BEMA-Nummern. Ersetzt das Planart-Label im Backend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leistung: Option<String>,
    /// Voll-HKP-EEBZ0-XML (Base64) des führenden Plans — Rendern per KZBV-XSLT.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ehkp_xml_b64: Option<String>,
    /// Alle Pläne des Falls (GAV + AAV) mit EBZ-Verlauf — für den Drawer.
    pub plans: Vec<HkpPlanEntry>,
}

/// Ein einzelner Plan innerhalb eines Falls (fürs Drawer).
#[derive(Debug, Clone, Serialize)]
pub struct HkpPlanEntry {
    pub plan_no: String,
    /// `GAV` (Regelversorgung/Kasse) | `AAV` (andersartig/privat).
    pub variant: String,
    /// Der führende GAV-Plan, der den Fall-Status bestimmt.
    pub is_primary: bool,
    pub planart: String,
    pub antragsnummer: String,
    /// Plan-Status (AAV ohne EBZ = `privat`).
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_on: Option<String>,
    /// EBZ-Verlauf dieses Plans (Anträge, Antworten, Rückfragen, Nachreichungen).
    pub submissions: Vec<HkpSubmission>,
}

/// Ein EBZ-Vorgang eines Plans (fürs Drawer-Timeline).
#[derive(Debug, Clone, Serialize)]
pub struct HkpSubmission {
    /// `antrag` | `antwort` | `rueckfrage` | `nachreichung`.
    pub kind: String,
    /// Relevantes Datum (`JJJJMMTT`).
    pub date: String,
    /// Bei Antworten: `genehmigt` | `abgelehnt`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// Nächtlicher Aggregat-Report fürs Modul „Praxis-Steuerung" (Controlling aus
/// der Z1-DB). Erstellt in [`crate::z1db::control`]; die Cloud upsertet die
/// Aggregate und speichert `sync.schema` + `sync.pending_mappings` (Grundlage
/// für die Spalten-Zuordnung am Piloten).
///
/// **Backend-Vertrag:** `POST /api/v1/connector/z1/control-report`.
#[derive(Debug, Clone, Serialize)]
pub struct ControlReport {
    pub sync: ControlSync,
    pub revenue: Vec<RevenueRow>,
    pub payments: Vec<PaymentRow>,
    pub ar_aging: Vec<ArAgingRow>,
    pub open_services: Vec<OpenServicesRow>,
}

/// Sync-Metadaten des Control-Reports.
#[derive(Debug, Clone, Serialize)]
pub struct ControlSync {
    /// Datenstand (ISO-Datum des Laufs).
    pub watermark: String,
    /// `ok` | `partial` (einzelne Teile ausgelassen) | `pending_mapping` (alle).
    pub status: String,
    /// Gescannte BEH-Quellzeilen im Zeitfenster (0, solange Mapping fehlt).
    pub rows_scanned: i64,
    /// Schema-Discovery `{tabelle: [spalten…]}` (INFORMATION_SCHEMA).
    pub schema: serde_json::Value,
    /// Ausgelassene Report-Teile: `{teil: {missing: [...], available: [...]}}`.
    pub pending_mappings: serde_json::Value,
}

/// Honorar je Monat × Art × Behandler (aus `BEH ⋈ LBLOCKENTRY`).
/// `gruppe`/`standort`/`eigenlabor`/`fremdlabor` sind `null`, bis die
/// entsprechenden Z1-Spalten am Piloten gemappt sind (nie erfinden).
#[derive(Debug, Clone, Serialize)]
pub struct RevenueRow {
    /// Monatserster als ISO-Datum, z. B. `"2026-07-01"`.
    pub period: String,
    /// `bema` | `goz` | `privat` (unbekannte Z1-Rohwerte kleingeschrieben durchgereicht).
    pub art: String,
    pub gruppe: Option<String>,
    pub behandler: String,
    pub standort: Option<String>,
    pub honorar: f64,
    pub eigenlabor: Option<f64>,
    pub fremdlabor: Option<f64>,
    pub n_leistungen: i64,
    pub n_faelle: i64,
}

/// Zahlungseingänge je Monat × Zahlart (aus `KONTO`/`CASH`).
#[derive(Debug, Clone, Serialize)]
pub struct PaymentRow {
    pub period: String,
    pub art: String,
    pub eingang: f64,
    pub n: i64,
}

/// Offene Forderungen je Alters-Bucket (`FAKT` − `KONTO`), Stichtags-Snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct ArAgingRow {
    /// ISO-Datum des Snapshots, z. B. `"2026-07-08"`.
    pub snapshot_date: String,
    /// `0-30` | `31-60` | `61-90` | `90+` (| `unbekannt` bei unlesbarem Datum).
    pub bucket: String,
    pub offen: f64,
    pub n: i64,
}

/// Erbrachte, nicht abgerechnete Leistungen je Behandler (`BEH` ohne `BILL`).
#[derive(Debug, Clone, Serialize)]
pub struct OpenServicesRow {
    pub snapshot_date: String,
    pub behandler: String,
    pub offen_betrag: f64,
    pub n: i64,
    /// Ältestes Leistungsdatum (ISO), sofern lesbar.
    pub oldest: Option<String>,
}

#[derive(Debug, Serialize)]
struct FiledBody<'a> {
    patient_id: &'a str,
    matched_by: &'a str,
}

#[derive(Debug, Serialize)]
struct FailedBody<'a> {
    reason: &'a str,
}

#[derive(Debug, Serialize)]
struct UnmatchedBody<'a> {
    reason: &'a str,
    /// Nahe Z1-PATNR-Kandidaten für die manuelle Zuordnung durch das Team.
    candidates: &'a [String],
}

#[derive(Debug, Serialize)]
struct Heartbeat<'a> {
    version: &'a str,
    vdds_registered: bool,
    kim_watching: bool,
    hkp_db_watching: bool,
    /// Capability: welche Dokumenttypen dieser Connector in die PVS-Akte ablegen
    /// darf. Quelle: [`crate::config::ConnectorConfig::supported_document_kinds`]
    /// — `anamnese`/`hkp` immer, `rechnung`/`storno` nur mit aktivem Modul
    /// „Rechnungen im PVS ablegen".
    ///
    /// **Cloud-Vertrag (im Walletpass-Repo umzusetzen, NICHT hier):**
    /// `GET /api/v1/connector/documents/pending` muss die Rückgabe auf die
    /// zuletzt gemeldeten `supported_document_kinds` des jeweiligen Connectors
    /// filtern — ein Beleg, dessen `kind` der installierte Connector nicht kann,
    /// wird gar nicht erst ausgeliefert (bleibt `pending`/geht in einen sichtbaren
    /// „Connector-Update nötig"-Zustand), statt über 5× `/failed`-Backoff still
    /// auf `failed` zu laufen. Bis der Filter steht, ist das Feld additiv und
    /// rückwärtskompatibel: eine alte Cloud ignoriert es einfach.
    supported_document_kinds: &'a [&'a str],
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<&'a str>,
}

/// Ein Cloud-Patient ohne Z1-PATID (aus `GET /connector/z1/patients/unmatched`),
/// den der Connector gegen die Z1-`PAT`-Tabelle matcht. Felder Z1-normalisiert.
#[derive(Debug, Clone, Deserialize)]
pub struct UnmatchedPatient {
    pub cloud_id: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub birth_name: String,
    #[serde(default)]
    pub birth_date: String,   // 'JJJJMMTT'
    #[serde(default)]
    pub postal_code: String,
    #[serde(default)]
    pub email: String,
}

/// Ein in Z1 gefundener Treffer, den der Connector zurückmeldet
/// (`POST /connector/z1/patients/matched`).
#[derive(Debug, Clone, Serialize)]
pub struct PatientMatch {
    pub cloud_id: String,
    pub patient_id: String,   // gefundene Z1-PATID
    pub matched_by: String,   // "name_dob" | "name_dob_plz"
}

impl CloudClient {
    pub fn new(cfg: &ConnectorConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("praxishub-connector/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(Self {
            http,
            base_url: cfg.praxishub_base_url.trim_end_matches('/').to_string(),
            tenant_id: cfg.tenant_id.clone(),
            api_key: cfg.api_key.clone(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1/connector/{}", self.base_url, path)
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.bearer_auth(&self.api_key)
            .header("X-Praxishub-Tenant", &self.tenant_id)
    }

    /// Erreichbarkeits-/Auth-Check. Gibt eine kurze Statusmeldung zurück.
    pub async fn ping(&self) -> Result<String> {
        let resp = self
            .auth(self.http.get(self.url("ping")))
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        if resp.status().is_success() {
            Ok("verbunden".into())
        } else {
            Err(ConnectorError::Http(format!("HTTP {}", resp.status())))
        }
    }

    pub async fn heartbeat(
        &self,
        vdds_registered: bool,
        kim_watching: bool,
        hkp_db_watching: bool,
        supported_document_kinds: &[&str],
        last_error: Option<&str>,
    ) -> Result<()> {
        let body = Heartbeat {
            version: env!("CARGO_PKG_VERSION"),
            vdds_registered,
            kim_watching,
            hkp_db_watching,
            supported_document_kinds,
            last_error,
        };
        self.auth(self.http.post(self.url("heartbeat")))
            .json(&body)
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Meldet eine genehmigte HKP/EBZ-Nachricht. Erfolg ⇒ Watcher darf die UIDL
    /// als „gesehen" markieren (sonst Retry im nächsten Zyklus).
    pub async fn report_hkp(&self, report: &HkpReport) -> Result<()> {
        self.auth(self.http.post(self.url("hkp")))
            .json(report)
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Meldet einen aus der Z1-DB gelesenen HKP-Fall (DB-Weg, ersetzt KIM).
    /// **Backend-Vertrag offen:** `POST /api/v1/connector/z1/hkp-status`.
    pub async fn report_hkp_case(&self, report: &HkpCaseReport) -> Result<()> {
        self.auth(self.http.post(self.url("z1/hkp-status")))
            .json(report)
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Meldet den nächtlichen Aggregat-Report der Praxis-Steuerung.
    /// **Backend-Vertrag:** `POST /api/v1/connector/z1/control-report`.
    pub async fn report_control(&self, report: &ControlReport) -> Result<()> {
        self.auth(self.http.post(self.url("z1/control-report")))
            .json(report)
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Potenzialanalyse (PVS-agnostische Rohzahlen + bewertete Befunde) fürs
    /// Analyse-Dashboard. **Backend-Vertrag offen:**
    /// `POST /api/v1/connector/pvs/potential-analysis`.
    pub async fn report_analysis(&self, report: &crate::analysis::PotentialReport) -> Result<()> {
        self.auth(self.http.post(self.url("pvs/potential-analysis")))
            .json(report)
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Cloud-Patienten ohne Z1-PATID zum Nachmatchen (seitenweise über `limit`).
    pub async fn fetch_unmatched_patients(&self, limit: u32) -> Result<Vec<UnmatchedPatient>> {
        let url = format!("{}?limit={}", self.url("z1/patients/unmatched"), limit);
        self.auth(self.http.get(url))
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .json::<Vec<UnmatchedPatient>>()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))
    }

    /// Meldet in Z1 gefundene PATIDs zurück (Bulk). Backend ergänzt nur leere Nummern.
    pub async fn report_patient_matches(&self, matches: &[PatientMatch]) -> Result<()> {
        if matches.is_empty() {
            return Ok(());
        }
        self.auth(self.http.post(self.url("z1/patients/matched")))
            .json(matches)
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Holt anstehende Rückschreib-Bündel (digitale Aufnahme → Z1).
    /// **Backend-Vertrag offen:** `GET /api/v1/connector/z1/writeback/pending`.
    pub async fn fetch_pending_writebacks(&self) -> Result<Vec<PendingWriteback>> {
        let resp = self
            .auth(self.http.get(self.url("z1/writeback/pending")))
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        resp.json::<Vec<PendingWriteback>>()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))
    }

    /// Quittiert ein erfolgreich in Z1 zurückgeschriebenes Bündel.
    /// **Backend-Vertrag offen:** `POST /api/v1/connector/z1/writeback/{id}/applied`.
    pub async fn ack_writeback_applied(
        &self,
        id: &str,
        patient_id: &str,
        report: Option<&WritebackReport>,
    ) -> Result<()> {
        let body = WritebackAppliedBody {
            patient_id,
            matched_by: "z1db",
            contact_updated: report.map(|r| r.contact_updated),
            address_updated: report.map(|r| r.address_updated),
            cave_appended: report.map(|r| r.cave_appended),
            co_appended: report.map(|r| r.co_appended),
            anamnese_inserted: report.map(|r| r.anamnese_inserted),
            notes_inserted: report.map(|r| r.notes_inserted),
            skipped: report.map(|r| r.skipped.clone()).unwrap_or_default(),
        };
        self.auth(
            self.http
                .post(self.url(&format!("z1/writeback/{id}/applied"))),
        )
        .json(&body)
        .send()
        .await
        .map_err(|e| ConnectorError::Http(e.to_string()))?
        .error_for_status()
        .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Meldet, dass ein Rückschreib-Bündel (noch) nicht angewandt werden konnte.
    /// **Backend-Vertrag offen:** `POST /api/v1/connector/z1/writeback/{id}/failed`.
    pub async fn ack_writeback_failed(&self, id: &str, reason: &str) -> Result<()> {
        self.auth(
            self.http
                .post(self.url(&format!("z1/writeback/{id}/failed"))),
        )
        .json(&FailedBody { reason })
        .send()
        .await
        .map_err(|e| ConnectorError::Http(e.to_string()))?
        .error_for_status()
        .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Meldet, dass der Patient **nicht sicher** zugeordnet werden konnte (nah dran,
    /// aber mehrdeutig) → gehört zur **manuellen Zuordnung** durch das Team. Liefert
    /// die nahen Kandidaten (Z1-PATNRs) mit. Das Backend soll den Fall aus der
    /// automatischen `pending`-Liste nehmen und dem Team mit **Signalwirkung** zeigen.
    /// **Backend-Vertrag offen:** `POST /api/v1/connector/z1/writeback/{id}/unmatched`.
    pub async fn ack_writeback_unmatched(
        &self,
        id: &str,
        reason: &str,
        candidates: &[String],
    ) -> Result<()> {
        self.auth(
            self.http
                .post(self.url(&format!("z1/writeback/{id}/unmatched"))),
        )
        .json(&UnmatchedBody { reason, candidates })
        .send()
        .await
        .map_err(|e| ConnectorError::Http(e.to_string()))?
        .error_for_status()
        .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Holt die aktuell zur PVS-Ablage anstehenden Dokumente.
    /// **Backend-Vertrag offen:** `GET /api/v1/connector/documents/pending`.
    pub async fn fetch_pending_documents(&self) -> Result<Vec<PendingDocument>> {
        let resp = self
            .auth(self.http.get(self.url("documents/pending")))
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        resp.json::<Vec<PendingDocument>>()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))
    }

    /// Quittiert einen erfolgreichen Z1-Import; das Backend nimmt das Dokument aus
    /// „pending" und hält die getroffene Z1-PATID fest („für genau diesen Patienten").
    /// `patient_id` = getroffene PATID (leer beim Name/Geburtsdatum-Match),
    /// `matched_by` = "patient_id" | "name_dob".
    pub async fn ack_document_filed(
        &self,
        id: &str,
        patient_id: &str,
        matched_by: &str,
    ) -> Result<()> {
        self.auth(self.http.post(self.url(&format!("documents/{id}/filed"))))
            .json(&FiledBody { patient_id, matched_by })
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }

    /// Meldet, dass der Z1-Import NICHT möglich war (mit Grund). Das Backend
    /// wiederholt mit Backoff und markiert das Dokument irgendwann als „failed".
    pub async fn ack_document_failed(&self, id: &str, reason: &str) -> Result<()> {
        self.auth(self.http.post(self.url(&format!("documents/{id}/failed"))))
            .json(&FailedBody { reason })
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        Ok(())
    }
}
