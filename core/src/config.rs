//! Per-Praxis-Konfiguration des Connectors.
//!
//! Persistiert als JSON im Per-User-AppData. `api_key` und `kim_password` werden
//! at-rest geschützt (Windows: DPAPI, an den Benutzer gebunden — siehe
//! [`crate::crypto`]); auf Nicht-Windows-Plattformen Klartext (nur Dev).

use crate::error::Result;
use crate::paths;
use serde::{Deserialize, Serialize};

fn default_base_url() -> String {
    "https://api.praxishub.ai".to_string()
}
fn default_kim_host() -> String {
    "127.0.0.1".to_string()
}
fn default_kim_port() -> u16 {
    995
}
fn default_poll() -> u64 {
    60
}
fn default_doc_poll() -> u64 {
    60
}
fn default_z1_database() -> String {
    "Z1".to_string()
}
fn default_hkp_lookback() -> u32 {
    24
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorConfig {
    #[serde(default = "default_base_url")]
    pub praxishub_base_url: String,
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default)]
    pub api_key: String,

    // KIM-Clientmodul (lokaler POP3-Proxy, liefert bereits entschlüsselt).
    #[serde(default = "default_kim_host")]
    pub kim_host: String,
    #[serde(default = "default_kim_port")]
    pub kim_port: u16,
    #[serde(default)]
    pub kim_user: String,
    #[serde(default)]
    pub kim_password: String,
    #[serde(default = "default_poll")]
    pub kim_poll_seconds: u64,

    /// Poll-Intervall des **Dokumenten-Push** (Variante B). Eigenständig, weil
    /// dieser Weg NICHT vom KIM-Postfach abhängt — er läuft auch, wenn KIM gerade
    /// nicht erreichbar ist.
    #[serde(default = "default_doc_poll")]
    pub doc_poll_seconds: u64,

    /// KIM-Clientmodule am localhost präsentieren oft selbstsignierte Zertifikate.
    #[serde(default = "default_true")]
    pub kim_allow_invalid_cert: bool,

    /// VDDS-Austausch-Verzeichnis (wo der Connector die `VDDS_MMO.INI` + das PDF
    /// für den PVS-Import ablegt). Leer = Windows-Temp. Am Z1 ggf. ein festes
    /// Abholverzeichnis eintragen. Siehe docs/OPERATIONS.md.
    #[serde(default)]
    pub exchange_dir: String,

    // ── Z1-SQL-Datenbank (Lesen + strukturiertes Rückschreiben) ──────────────
    // Voller Kontext: docs/Z1-DATABASE.md. Der Connector nutzt einen dedizierten
    // Read-only-Login (`praxishub_ro`); die Rückschreib-Funktionen brauchen einen
    // schreibfähigen Login (z. B. den Z1-App-Login) — siehe `z1_db_write_user`.
    /// SQL-Server-Instanz, z. B. `srv-fs\z1` (Host `\` Named Instance).
    #[serde(default)]
    pub z1_db_server: String,
    #[serde(default = "default_z1_database")]
    pub z1_db_database: String,
    /// Read-only-Login (`db_datareader`) — Status-/HKP-/Stammdaten-Lesen.
    #[serde(default)]
    pub z1_db_user: String,
    #[serde(default)]
    pub z1_db_password: String,
    /// Schreibfähiger Login für das Rückschreiben (Kontakt/CAVE/Anamnese).
    /// Leer ⇒ Rückschreiben deaktiviert. DPAPI-geschützt wie die übrigen Secrets.
    #[serde(default)]
    pub z1_db_write_user: String,
    #[serde(default)]
    pub z1_db_write_password: String,
    /// Selbstsigniertes Serverzertifikat der Z1-Instanz akzeptieren (Standard: ja).
    #[serde(default = "default_true")]
    pub z1_db_trust_cert: bool,
    /// Effizienz-Grenze fürs HKP-Tracking: **abgeschlossene/abgelehnte** Fälle werden
    /// nur gemeldet, wenn ihr Abschluss ≤ so viele Monate zurückliegt. **Offene und
    /// abgelaufene** Fälle werden IMMER gemeldet (Werthebel). `0` = unbegrenzt. Das FE
    /// filtert die Anzeige feiner (Standard z. B. 6 Monate, einstellbar).
    #[serde(default = "default_hkp_lookback")]
    pub z1_hkp_lookback_months: u32,

    // ── Rückschreib-Toggles (jede Fähigkeit einzeln aktivierbar) ─────────────
    /// Kontaktdaten (Telefon/E-Mail) in `ADR` zurückschreiben.
    #[serde(default)]
    pub writeback_contact: bool,
    /// Adresse (Straße/Hausnr./PLZ/Ort) in `ADR` **überschreiben**, wenn der
    /// Patient abweichende Angaben macht.
    #[serde(default)]
    pub writeback_address: bool,
    /// CAVE/Allergien additiv an die Risikoanamnese (`PAT.ANAMNESE`) anhängen.
    #[serde(default)]
    pub writeback_cave: bool,
    /// Krankenanamnese als Zeilen in `PATINFO` (ART=1) schreiben — wie Nelly.
    #[serde(default)]
    pub writeback_anamnese: bool,
    /// Neupatienten anlegen (Vorab-Aufnahme). **Vorsicht:** Dubletten-Risiko beim
    /// Kartenstecken — nur nach empirischem Karten-Match-Test aktivieren.
    #[serde(default)]
    pub writeback_new_patient: bool,
    /// Bei „c/o"-Adresszusatz (CO/co/c/o) den festen Hinweis `c/o Adresse`
    /// (wortwörtlich, als Flag) in die Risikoanamnese (`PAT.ANAMNESE`) schreiben.
    #[serde(default)]
    pub writeback_co_to_risk: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            praxishub_base_url: default_base_url(),
            tenant_id: String::new(),
            api_key: String::new(),
            kim_host: default_kim_host(),
            kim_port: default_kim_port(),
            kim_user: String::new(),
            kim_password: String::new(),
            kim_poll_seconds: default_poll(),
            doc_poll_seconds: default_doc_poll(),
            kim_allow_invalid_cert: true,
            exchange_dir: String::new(),
            z1_db_server: String::new(),
            z1_db_database: default_z1_database(),
            z1_db_user: String::new(),
            z1_db_password: String::new(),
            z1_db_write_user: String::new(),
            z1_db_write_password: String::new(),
            z1_db_trust_cert: true,
            z1_hkp_lookback_months: default_hkp_lookback(),
            writeback_contact: false,
            writeback_address: false,
            writeback_cave: false,
            writeback_anamnese: false,
            writeback_new_patient: false,
            writeback_co_to_risk: false,
        }
    }
}

impl ConnectorConfig {
    /// Austausch-Verzeichnis für VDDS-media (konfiguriert oder Windows-Temp).
    pub fn exchange_dir_path(&self) -> std::path::PathBuf {
        if self.exchange_dir.trim().is_empty() {
            std::env::temp_dir()
        } else {
            std::path::PathBuf::from(self.exchange_dir.trim())
        }
    }
}

impl ConnectorConfig {
    /// Lädt die Konfiguration; bei fehlender Datei werden Defaults zurückgegeben.
    /// Geschützte Secrets (DPAPI) werden entschlüsselt zurückgegeben.
    pub fn load() -> Result<Self> {
        let path = paths::config_file()?;
        match std::fs::read(&path) {
            Ok(bytes) => {
                let mut cfg: Self = serde_json::from_slice(&bytes)?;
                cfg.api_key = crate::crypto::unprotect(&cfg.api_key);
                cfg.kim_password = crate::crypto::unprotect(&cfg.kim_password);
                cfg.z1_db_password = crate::crypto::unprotect(&cfg.z1_db_password);
                cfg.z1_db_write_password = crate::crypto::unprotect(&cfg.z1_db_write_password);
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Speichert die Konfiguration; Secrets werden vor dem Schreiben geschützt
    /// (Windows: DPAPI, sonst Klartext).
    pub fn save(&self) -> Result<()> {
        let path = paths::config_file()?;
        let mut on_disk = self.clone();
        on_disk.api_key = crate::crypto::protect(&self.api_key);
        on_disk.kim_password = crate::crypto::protect(&self.kim_password);
        on_disk.z1_db_password = crate::crypto::protect(&self.z1_db_password);
        on_disk.z1_db_write_password = crate::crypto::protect(&self.z1_db_write_password);
        let json = serde_json::to_vec_pretty(&on_disk)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Hinreichend konfiguriert, um den KIM-Watcher zu starten?
    pub fn kim_ready(&self) -> bool {
        !self.kim_host.is_empty()
            && self.kim_port != 0
            && !self.kim_user.is_empty()
            && !self.kim_password.is_empty()
    }

    /// Hinreichend konfiguriert, um mit der Cloud zu sprechen?
    pub fn cloud_ready(&self) -> bool {
        !self.praxishub_base_url.is_empty() && !self.tenant_id.is_empty() && !self.api_key.is_empty()
    }

    /// Genug für read-only-Zugriff auf die Z1-DB (Status/HKP/Stammdaten lesen)?
    pub fn z1db_read_ready(&self) -> bool {
        !self.z1_db_server.is_empty()
            && !self.z1_db_database.is_empty()
            && !self.z1_db_user.is_empty()
            && !self.z1_db_password.is_empty()
    }

    /// Genug, um strukturiert zurückzuschreiben? Braucht einen schreibfähigen Login
    /// **und** mindestens einen aktiven Rückschreib-Toggle.
    pub fn z1db_write_ready(&self) -> bool {
        !self.z1_db_server.is_empty()
            && !self.z1_db_database.is_empty()
            && !self.z1_db_write_user.is_empty()
            && !self.z1_db_write_password.is_empty()
            && self.any_writeback_enabled()
    }

    /// Ist mindestens ein Rückschreib-Toggle aktiv?
    pub fn any_writeback_enabled(&self) -> bool {
        self.writeback_contact
            || self.writeback_address
            || self.writeback_cave
            || self.writeback_anamnese
            || self.writeback_new_patient
            || self.writeback_co_to_risk
    }
}
