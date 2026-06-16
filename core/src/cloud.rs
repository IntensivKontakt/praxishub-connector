//! HTTPS-Anbindung an die Praxishub-Cloud.
//!
//! Auth wie bei der Doctolib-Extension: `Authorization: Bearer <api_key>` +
//! `X-Praxishub-Tenant`. Endpunkte unter `/api/v1/connector/*`.
//!
//! **Backend-Arbeit offen:** diese Routen müssen in der Praxishub-API noch
//! angelegt werden (heartbeat / hkp). Vertrag siehe [`HkpReport`].

use crate::config::ConnectorConfig;
use crate::error::{ConnectorError, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

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

/// Ein fertiges PDF (Anamnese/HKP), das in die PVS-Akte abzulegen ist.
/// Cloud → Connector (`GET /connector/documents/pending`).
#[derive(Debug, Clone, Deserialize)]
pub struct PendingDocument {
    /// Idempotenz-/Quittungs-Schlüssel (an `/filed` bzw. `/failed` zurückgeben).
    pub id: String,
    /// "anamnese" | "hkp".
    pub kind: String,
    /// Z1-PATID, falls bekannt; sonst leer (dann Match über Name+Geburtsdatum).
    #[serde(default)]
    pub patient_id: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub first_name: String,
    /// Geburtsdatum `TT.MM.JJJJ`.
    #[serde(default)]
    pub birth_date: String,
    /// Das fertige PDF, Base64.
    pub pdf_base64: String,
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
struct Heartbeat<'a> {
    version: &'a str,
    vdds_registered: bool,
    kim_watching: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<&'a str>,
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
        last_error: Option<&str>,
    ) -> Result<()> {
        let body = Heartbeat {
            version: env!("CARGO_PKG_VERSION"),
            vdds_registered,
            kim_watching,
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

    /// Holt alle noch nicht abgelegten Dokumente (PDFs) dieses Tenants.
    /// Leeres Array, wenn nichts ansteht.
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

    /// Quittiert einen erfolgreichen Z1-Import. `patient_id` = die Z1-PATID, gegen
    /// die tatsächlich abgelegt wurde; `matched_by` = "patient_id" | "name_dob".
    pub async fn mark_document_filed(
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

    /// Meldet, dass das Dokument NICHT in Z1 abgelegt werden konnte (mit Grund).
    /// Danach erscheint es nicht mehr unter `/pending`.
    pub async fn mark_document_failed(&self, id: &str, reason: &str) -> Result<()> {
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
