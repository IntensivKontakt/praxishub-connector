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
fn default_control_hour() -> u8 {
    3
}
fn default_control_months() -> u32 {
    36
}
fn default_upload_typenr() -> u32 {
    24 // VDDS Tabelle 15 „Allgemeines Dokument" — neutraler Bucket für Patienten-Uploads.
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
    /// € je BEMA-Punkt für die ePAR-Honorar-Schätzung (KZV-Punktwert PAR).
    /// Der ePAR-Antrag enthält keine Euro-Beträge — bei gesetztem Punktwert
    /// meldet der Connector Betrag = geplante BEMA-Punkte × Punktwert.
    /// `0` (Default) = keine Schätzung; Leistungen + Punktesumme kommen immer.
    #[serde(default)]
    pub z1_par_punktwert: f64,

    // ── Praxis-Steuerung (nächtlicher Aggregat-Sync, docs: z1db/control.rs) ──
    /// Täglichen Controlling-Sync (BEH/KONTO/CASH/FAKT/BILL-Aggregate → Cloud)
    /// aktivieren. Opt-in, Default aus.
    #[serde(default)]
    pub z1_control_enabled: bool,
    /// FRÜHESTE lokale Stunde des täglichen Laufs (Default 3 = 03:00). Der Sync läuft
    /// einmal/Tag am oder nach dieser Stunde beim ersten Mal, wenn der PC an ist — war
    /// er nachts aus, wird morgens nachgeholt (Anacron). Muss ≤ Öffnungszeit sein.
    #[serde(default = "default_control_hour")]
    pub z1_control_hour: u8,
    /// Zeitfenster der Monats-Aggregate (revenue/payments/open_services).
    #[serde(default = "default_control_months")]
    pub z1_control_months: u32,
    /// Spaltennamen-Overrides für die Controlling-Aggregate (JSON-Objekt,
    /// Feldname → Z1-Spaltenname) — finalisiert die Zuordnung am Piloten ohne
    /// Neubau. `None` = Default-Vermutungen (siehe `z1db::control::ColumnMap`).
    #[serde(default)]
    pub z1_control_column_map: Option<serde_json::Value>,

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
    /// Karteikarten-Notizen (z. B. Rechnungsstatus „… bezahlt") als `BEH`-Freitext-
    /// zeilen schreiben (GOART leer, `BEHTEXTART='k'` = Verlaufsdoku, NICHT
    /// abrechnungsrelevant). Eigener Kanal, getrennt von `writeback_anamnese` —
    /// damit Rechnungsstatus NICHT in der Krankenanamnese landet. Braucht den
    /// schreibfähigen Login. Siehe `z1db::writeback::write_notes`.
    #[serde(default)]
    pub writeback_notes: bool,
    /// Neupatienten anlegen (Vorab-Aufnahme). **Vorsicht:** Dubletten-Risiko beim
    /// Kartenstecken — nur nach empirischem Karten-Match-Test aktivieren.
    #[serde(default)]
    pub writeback_new_patient: bool,
    /// Bei „c/o"-Adresszusatz (CO/co/c/o) den festen Hinweis `c/o Adresse`
    /// (wortwörtlich, als Flag) in die Risikoanamnese (`PAT.ANAMNESE`) schreiben.
    #[serde(default)]
    pub writeback_co_to_risk: bool,
    /// Nach dem VDDS-Import die Z1-`ARCHIV`-Indexzeile schreiben — macht das
    /// abgelegte Dokument im Z1-Karteireiter „Archiv" sichtbar. Braucht den
    /// schreibfähigen Login. Siehe `z1db::archiv`.
    #[serde(default)]
    pub writeback_archiv_link: bool,
    /// Modul „Rechnungen im PVS ablegen": Rechnungs-/Storno-Belege aus dem
    /// Praxishub-Rechnungsmodul ins PVS-Archiv legen **und** den Zahlungsstatus als
    /// Karteikarten-Notiz vermerken. Steuert, welche Dokumenttypen der Connector im
    /// Heartbeat als unterstützt meldet (`supported_document_kinds`); ist er aus,
    /// liefert die Cloud gar keine Belege. Aktiviert automatisch `writeback_notes`.
    #[serde(default)]
    pub pvs_file_invoices: bool,
    /// VDDS-Objekttyp (Tabelle 15) für vom Patienten in der Anamnese hochgeladene
    /// Dateien (`anamnese_upload`: Röntgen/Foto/Medikationsplan). Steuert die
    /// Kategorie in PraxisArchiv/Z1-Archiv. Default 24 („Allgemeines Dokument");
    /// NICHT 13 (Anamnesebogen). Alternativen laut Praxis-Bestand: 8 Bild/Foto,
    /// 7 Foto, 11 Fremdbefund.
    #[serde(default = "default_upload_typenr")]
    pub upload_document_typenr: u32,
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
            z1_par_punktwert: 0.0,
            z1_control_enabled: false,
            z1_control_hour: default_control_hour(),
            z1_control_months: default_control_months(),
            z1_control_column_map: None,
            writeback_contact: false,
            writeback_address: false,
            writeback_cave: false,
            writeback_anamnese: false,
            writeback_notes: false,
            writeback_new_patient: false,
            writeback_co_to_risk: false,
            writeback_archiv_link: false,
            pvs_file_invoices: false,
            upload_document_typenr: default_upload_typenr(),
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
            || self.writeback_notes
            || self.pvs_file_invoices
            || self.writeback_new_patient
            || self.writeback_co_to_risk
    }

    /// Notizen-Rückschreiben aktiv? Das Modul „Rechnungen im PVS ablegen"
    /// (`pvs_file_invoices`) aktiviert den Notiz-Kanal automatisch mit.
    pub fn writeback_notes_enabled(&self) -> bool {
        self.writeback_notes || self.pvs_file_invoices
    }

    /// Archiv-Indexierung aktiv? Das Modul „Rechnungen im PVS ablegen" aktiviert
    /// die Archiv-Anzeige automatisch mit — sonst läge der Rechnungsbeleg nur im
    /// PraxisArchiv, aber nicht sichtbar im Z1-Karteireiter „Archiv".
    pub fn archiv_link_enabled(&self) -> bool {
        self.writeback_archiv_link || self.pvs_file_invoices
    }

    /// Dokumenttypen, die der Connector der Cloud als ablegbar meldet
    /// (`supported_document_kinds` im Heartbeat). Anamnese/HKP immer; Rechnung/
    /// Storno nur mit aktivem Modul „Rechnungen im PVS ablegen" — sonst liefert
    /// die Cloud gar keine Belege dieses Typs aus.
    pub fn supported_document_kinds(&self) -> Vec<&'static str> {
        // Anamnese/HKP/Patienten-Uploads gehören zum Aufnahme-Kernfluss (immer an).
        let mut kinds = vec!["anamnese", "hkp", "anamnese_upload"];
        if self.pvs_file_invoices {
            kinds.push("rechnung");
            kinds.push("storno");
        }
        kinds
    }

    /// Schreibfähiger Z1-Login konfiguriert (unabhängig von den Toggles)?
    /// Für die ARCHIV-Verlinkung, die in der Dokumenten-Schleife läuft und
    /// keinen Writeback-Loop braucht.
    pub fn z1db_write_login_configured(&self) -> bool {
        !self.z1_db_server.is_empty()
            && !self.z1_db_database.is_empty()
            && !self.z1_db_write_user.is_empty()
            && !self.z1_db_write_password.is_empty()
    }
}
