//! Strukturiertes Rückschreiben in die Z1-DB — die bei der digitalen Aufnahme
//! gesammelten Daten in die Patientenakte übernehmen.
//!
//! Jede Fähigkeit ist über einen Config-Toggle einzeln aktivierbar
//! ([`ConnectorConfig`]). Verifizierte Schreibpfade (siehe `docs/Z1-DATABASE.md`):
//!   * Kontakt (`writeback_contact`)  → `UPDATE ADR` TELEFON1/SECUREMAIL
//!   * Adresse (`writeback_address`)  → `UPDATE ADR` STR/PLZ/ORT (überschreibend)
//!   * CAVE    (`writeback_cave`)     → additiv an `PAT.ANAMNESE` (Risikoanamnese)
//!   * Anamnese(`writeback_anamnese`) → `INSERT INTO PATINFO` (ART=1, wie Nelly)

use crate::cloud::{CloudClient, PendingWriteback};
use crate::config::ConnectorConfig;
use crate::error::{ConnectorError, Result};
use crate::paths;
use crate::z1db::client::{fresh_rinfo, pad_left, Z1Connection};
use crate::z1db::{self, LoopHandle};
use chrono::Local;
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Risikoanamnese-Feld `PAT.ANAMNESE` ist `varchar(80)`.
const ANAMNESE_MAX: usize = 80;
/// `PATINFO.INFORMATION` ist `varchar(80)`.
const PATINFO_INFO_MAX: usize = 80;
/// ART des „Anamnese"-Tabs in der Z1-Patienten-Information.
const ART_ANAMNESE: &str = "1";
/// Default-Behandler (`LEBID`) für maschinell erzeugte Anamnese-Zeilen.
/// TODO: praxisseitig konfigurierbar machen (analog Nelly nutzt Z1 hier den
/// erfassenden Behandler).
const DEFAULT_LEBID: &str = " 15";

/// Kontakt-/Adressdaten aus der digitalen Aufnahme. `None` = kein Wert geliefert
/// (Feld unangetastet lassen).
#[derive(Debug, Clone, Default)]
pub struct ContactData {
    pub phone: Option<String>,
    pub email: Option<String>,
    /// Straße **inkl. Hausnummer** (Z1 `ADR.STR` hält beides zusammen).
    pub street: Option<String>,
    pub zip: Option<String>,
    pub city: Option<String>,
}

/// Ein Bündel zurückzuschreibender Patientendaten (eine Aufnahme).
#[derive(Debug, Clone)]
pub struct PatientWriteback {
    /// Z1-`PATNR` (mit/ohne Padding — wird intern normalisiert).
    pub patient_id: String,
    pub contact: Option<ContactData>,
    /// CAVE-/Allergie-Einträge — werden additiv an die Risikoanamnese gehängt.
    pub cave: Vec<String>,
    /// Krankenanamnese-Zeilen — je Zeile ein `PATINFO`-Eintrag (ART=1).
    pub anamnese: Vec<String>,
}

/// Was tatsächlich geschrieben wurde (für Logging/Ack an die Cloud).
#[derive(Debug, Default, Clone)]
pub struct WritebackReport {
    pub contact_updated: bool,
    pub address_updated: bool,
    pub cave_appended: usize,
    pub anamnese_inserted: usize,
    /// Nicht ausgeführte Teile (Toggle aus, Feld zu lang, Adresse geteilt …).
    pub skipped: Vec<String>,
}

/// Wendet ein Rückschreib-Bündel gemäß den aktiven Toggles an. Ein fehlgeschlagener
/// Teilschritt bricht NICHT den ganzen Vorgang ab — er wird protokolliert; die
/// übrigen Teile laufen weiter (Robustheit vor Vollständigkeit).
pub async fn apply_writeback(
    conn: &mut Z1Connection,
    cfg: &ConnectorConfig,
    data: &PatientWriteback,
) -> Result<WritebackReport> {
    let patnr = data.patient_id.trim().to_string();
    if patnr.is_empty() {
        return Err(ConnectorError::Z1Db("Rückschreiben ohne PATNR".into()));
    }
    let mut report = WritebackReport::default();

    if let Some(contact) = &data.contact {
        if cfg.writeback_contact || cfg.writeback_address {
            match write_contact(conn, cfg, &patnr, contact).await {
                Ok((c, a)) => {
                    report.contact_updated = c;
                    report.address_updated = a;
                }
                Err(e) => {
                    warn!(%patnr, error=%e, "Kontakt/Adresse-Rückschreiben fehlgeschlagen");
                    report.skipped.push(format!("Kontakt/Adresse: {e}"));
                }
            }
        } else {
            report.skipped.push("Kontakt/Adresse: Toggle aus".into());
        }
    }

    if !data.cave.is_empty() {
        if cfg.writeback_cave {
            match append_cave(conn, &patnr, &data.cave).await {
                Ok(n) => report.cave_appended = n,
                Err(e) => {
                    warn!(%patnr, error=%e, "CAVE-Rückschreiben fehlgeschlagen");
                    report.skipped.push(format!("CAVE: {e}"));
                }
            }
        } else {
            report.skipped.push("CAVE: Toggle aus".into());
        }
    }

    if !data.anamnese.is_empty() {
        if cfg.writeback_anamnese {
            match write_anamnese(conn, &patnr, &data.anamnese).await {
                Ok(n) => report.anamnese_inserted = n,
                Err(e) => {
                    warn!(%patnr, error=%e, "Anamnese-Rückschreiben fehlgeschlagen");
                    report.skipped.push(format!("Anamnese: {e}"));
                }
            }
        } else {
            report.skipped.push("Anamnese: Toggle aus".into());
        }
    }

    info!(
        %patnr, contact=report.contact_updated, address=report.address_updated,
        cave=report.cave_appended, anamnese=report.anamnese_inserted,
        "Z1-Rückschreiben abgeschlossen"
    );
    Ok(report)
}

/// Ermittelt die private Adress-ID (`PAT.ADRIDP`) und stellt sicher, dass sie
/// **nur** von diesem Patienten genutzt wird (kein geteilter Familien-Datensatz).
async fn adrid_for_patient(conn: &mut Z1Connection, patnr: &str) -> Result<String> {
    let adrid = conn
        .scalar_string(
            "SELECT LTRIM(RTRIM(ADRIDP)) FROM PAT WHERE LTRIM(RTRIM(PATNR)) = @P1",
            &[&patnr],
        )
        .await?
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ConnectorError::Z1Db(format!("Kein ADRIDP für PATNR {patnr}")))?;

    let shared = conn
        .scalar_i32(
            "SELECT COUNT(*) FROM PAT WHERE LTRIM(RTRIM(ADRIDP)) = @P1 \
             OR LTRIM(RTRIM(ADRIDR)) = @P1 OR LTRIM(RTRIM(ADRIDA)) = @P1 \
             OR LTRIM(RTRIM(ADRIDK)) = @P1",
            &[&adrid],
        )
        .await?;
    if shared != 1 {
        return Err(ConnectorError::Z1Db(format!(
            "Adress-Datensatz {adrid} wird von {shared} Patienten genutzt — nicht überschrieben"
        )));
    }
    Ok(adrid)
}

/// Aktualisiert Kontakt- (`TELEFON1`/`SECUREMAIL`) und/oder Adressfelder
/// (`STR`/`PLZ`/`ORT`) im bestehenden `ADR`-Datensatz. Gibt zurück, ob Kontakt-
/// bzw. Adressfelder tatsächlich geschrieben wurden.
async fn write_contact(
    conn: &mut Z1Connection,
    cfg: &ConnectorConfig,
    patnr: &str,
    contact: &ContactData,
) -> Result<(bool, bool)> {
    let adrid = adrid_for_patient(conn, patnr).await?;

    // Dynamische SET-Liste nur aus den erlaubten + gelieferten Feldern.
    let mut cols: Vec<&str> = Vec::new();
    let mut vals: Vec<String> = Vec::new();
    let mut contact_written = false;
    let mut address_written = false;

    if cfg.writeback_contact {
        if let Some(p) = contact.phone.as_ref().filter(|s| !s.trim().is_empty()) {
            cols.push("TELEFON1");
            vals.push(p.clone());
            contact_written = true;
        }
        if let Some(e) = contact.email.as_ref().filter(|s| !s.trim().is_empty()) {
            cols.push("SECUREMAIL");
            vals.push(e.clone());
            contact_written = true;
        }
    }
    if cfg.writeback_address {
        // Überschreibend: Straße/Hausnr., PLZ, Ort.
        if let Some(s) = contact.street.as_ref().filter(|s| !s.trim().is_empty()) {
            cols.push("STR");
            vals.push(s.clone());
            address_written = true;
        }
        if let Some(z) = contact.zip.as_ref().filter(|s| !s.trim().is_empty()) {
            cols.push("PLZ");
            vals.push(z.clone());
            address_written = true;
        }
        if let Some(o) = contact.city.as_ref().filter(|s| !s.trim().is_empty()) {
            cols.push("ORT");
            vals.push(o.clone());
            address_written = true;
        }
    }
    if cols.is_empty() {
        return Ok((false, false));
    }

    let old_rinfo = conn
        .scalar_string(
            "SELECT RINFO FROM ADR WHERE LTRIM(RTRIM(ADRID)) = @P1",
            &[&adrid],
        )
        .await?;
    let rinfo = fresh_rinfo(old_rinfo.as_deref());

    // SET feld=@P1, … , RINFO=@Pn  WHERE ADRID=@Pn+1
    let set_clause = cols
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{c} = @P{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let rinfo_idx = cols.len() + 1;
    let adrid_idx = cols.len() + 2;
    let sql = format!(
        "UPDATE ADR SET {set_clause}, RINFO = @P{rinfo_idx} WHERE LTRIM(RTRIM(ADRID)) = @P{adrid_idx}"
    );

    let mut params: Vec<&dyn tiberius::ToSql> = Vec::with_capacity(cols.len() + 2);
    for v in &vals {
        params.push(v);
    }
    params.push(&rinfo);
    params.push(&adrid);

    conn.exec_expect(&sql, &params, 1).await?;
    Ok((contact_written, address_written))
}

/// Hängt CAVE-Einträge additiv an die Risikoanamnese (`PAT.ANAMNESE`) an — es
/// wird **nie** gelöscht. Respektiert das `varchar(80)`-Limit; was nicht mehr
/// passt, wird ausgelassen (und der Aufrufer im Report informiert).
async fn append_cave(conn: &mut Z1Connection, patnr: &str, entries: &[String]) -> Result<usize> {
    let (old, old_rinfo) = {
        let old = conn
            .scalar_string(
                "SELECT ISNULL(ANAMNESE, '') FROM PAT WHERE LTRIM(RTRIM(PATNR)) = @P1",
                &[&patnr],
            )
            .await?
            .ok_or_else(|| ConnectorError::Z1Db(format!("PATNR {patnr} nicht gefunden")))?;
        let rinfo = conn
            .scalar_string(
                "SELECT RINFO FROM PAT WHERE LTRIM(RTRIM(PATNR)) = @P1",
                &[&patnr],
            )
            .await?;
        (old, rinfo)
    };

    let mut text = old;
    let mut appended = 0usize;
    for entry in entries {
        let e = entry.trim();
        if e.is_empty() {
            continue;
        }
        let addition = if text.is_empty() {
            format!("CAVE: {e}")
        } else {
            format!(" | CAVE: {e}")
        };
        if text.len() + addition.len() > ANAMNESE_MAX {
            warn!(%patnr, "CAVE-Eintrag passt nicht mehr in Risikoanamnese (80 Zeichen) — ausgelassen");
            break;
        }
        text.push_str(&addition);
        appended += 1;
    }
    if appended == 0 {
        return Ok(0);
    }

    let rinfo = fresh_rinfo(old_rinfo.as_deref());
    conn.exec_expect(
        "UPDATE PAT SET ANAMNESE = @P1, RINFO = @P2 WHERE LTRIM(RTRIM(PATNR)) = @P3",
        &[&text, &rinfo, &patnr],
        1,
    )
    .await?;
    Ok(appended)
}

/// Schreibt Krankenanamnese-Zeilen als `PATINFO`-Einträge (ART=1) — dasselbe
/// Muster wie Nellys Anamnese-Import (`PID='  0'`, keine `NUMBERPOOL`-Vergabe).
/// Läuft in einer Transaktion; alle Zeilen oder keine.
async fn write_anamnese(conn: &mut Z1Connection, patnr: &str, lines: &[String]) -> Result<usize> {
    let clean: Vec<String> = lines
        .iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if clean.is_empty() {
        return Ok(0);
    }

    let datum = Local::now().format("%Y%m%d").to_string();
    let patnr10 = pad_left(patnr, 10);

    // Nächste freie Zeilennummer für (PATNR, heute, ART=1).
    let mut next = conn
        .scalar_i32(
            "SELECT ISNULL(MAX(CAST(LTRIM(LFDPATINFOART) AS INT)), -1) + 1 FROM PATINFO \
             WHERE LTRIM(RTRIM(PATNR)) = @P1 AND DATUM = @P2 AND ART = @P3",
            &[&patnr, &datum, &ART_ANAMNESE],
        )
        .await?;

    conn.simple("BEGIN TRANSACTION").await?;
    let result = async {
        let mut inserted = 0usize;
        for line in &clean {
            let lfd = pad_left(&next.to_string(), 4);
            let rinfo = fresh_rinfo(None);
            let mut info = line.clone();
            if info.len() > PATINFO_INFO_MAX {
                info.truncate(PATINFO_INFO_MAX);
            }
            // Feste Felder wie Nelly: LEBID=Behandler, ART=1, PID='  0', Rest leer.
            conn.exec_expect(
                "INSERT INTO PATINFO \
                 (RINFO,PATNR,DATUM,LFDPATINFOART,LEBID,ART,INFORMATION,MDID,STATUS,PID,\
                  FARBE,TERMIN,PLANUNGSART,LFDANAMNESE,LFDFREITEXT,LFDANAMVERSION,\
                  FRAGEBOGENART,LFDFRAGEBOGEN,LFDFRAGEBOGENENTRY) \
                 VALUES (@P1,@P2,@P3,@P4,@P5,@P6,@P7,'','','  0','','','','','','','','','')",
                &[
                    &rinfo,
                    &patnr10,
                    &datum,
                    &lfd,
                    &DEFAULT_LEBID,
                    &ART_ANAMNESE,
                    &info,
                ],
                1,
            )
            .await?;
            inserted += 1;
            next += 1;
        }
        Ok::<usize, ConnectorError>(inserted)
    }
    .await;

    match result {
        Ok(n) => {
            conn.simple("COMMIT").await?;
            Ok(n)
        }
        Err(e) => {
            let _ = conn.simple("ROLLBACK").await;
            Err(e)
        }
    }
}

// ── Cloud-Anbindung: Aufnahme-Bündel ziehen und zurückschreiben ──────────────

/// Persistenter Store bereits angewandter Bündel-IDs (Idempotenz: verhindert
/// Doppel-Anwendung von CAVE/Anamnese, falls der Cloud-Ack mal fehlschlägt).
struct AppliedStore {
    set: HashSet<String>,
}

impl AppliedStore {
    fn load() -> Self {
        let set = paths::writeback_seen_store_file()
            .ok()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { set }
    }
    fn contains(&self, id: &str) -> bool {
        self.set.contains(id)
    }
    fn insert(&mut self, id: String) {
        if self.set.insert(id) {
            if let Ok(p) = paths::writeback_seen_store_file() {
                let _ = serde_json::to_vec(&self.set).map(|b| std::fs::write(p, b));
            }
        }
    }
}

impl From<&PendingWriteback> for PatientWriteback {
    fn from(w: &PendingWriteback) -> Self {
        let contact = ContactData {
            phone: w.phone.clone(),
            email: w.email.clone(),
            street: w.street.clone(),
            zip: w.zip.clone(),
            city: w.city.clone(),
        };
        let has_contact = contact.phone.is_some()
            || contact.email.is_some()
            || contact.street.is_some()
            || contact.zip.is_some()
            || contact.city.is_some();
        PatientWriteback {
            patient_id: w.patient_id.clone(),
            contact: has_contact.then_some(contact),
            cave: w.cave.clone(),
            anamnese: w.anamnese.clone(),
        }
    }
}

/// Startet die Rückschreib-Schleife als eigenständigen Task. Läuft nur, wenn ein
/// schreibfähiger Login + mindestens ein Toggle aktiv sind (`z1db_write_ready`).
pub fn spawn(cfg: ConnectorConfig) -> LoopHandle {
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        let cloud = match CloudClient::new(&cfg) {
            Ok(c) => c,
            Err(e) => {
                warn!(error=%e, "Writeback: Cloud-Client fehlgeschlagen — Schleife beendet");
                return;
            }
        };
        let mut applied = AppliedStore::load();
        let period = Duration::from_secs(cfg.doc_poll_seconds.max(30));
        let mut ticker = tokio::time::interval(period);
        info!(period_s = period.as_secs(), "Z1-Writeback-Schleife gestartet");
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(e) = run_cycle(&cfg, &cloud, &mut applied).await {
                        debug!(error=%e, "Writeback-Zyklus fehlgeschlagen");
                    }
                }
                _ = rx.changed() => {
                    if *rx.borrow() { info!("Z1-Writeback-Schleife gestoppt"); break; }
                }
            }
        }
    });
    LoopHandle::new(tx, join)
}

/// Ein Zyklus: anstehende Bündel holen und (soweit auflösbar) zurückschreiben.
async fn run_cycle(
    cfg: &ConnectorConfig,
    cloud: &CloudClient,
    applied: &mut AppliedStore,
) -> Result<()> {
    let pending = cloud.fetch_pending_writebacks().await?;
    if pending.is_empty() {
        return Ok(());
    }
    // Schreibfähige Verbindung (kann auch lesen → Patienten-Lookup).
    let mut conn = z1db::connect(
        &cfg.z1_db_server,
        &cfg.z1_db_database,
        &cfg.z1_db_write_user,
        &cfg.z1_db_write_password,
        cfg.z1_db_trust_cert,
    )
    .await?;

    for wb in &pending {
        // Schon angewandt (Ack war evtl. fehlgeschlagen) → nur Ack nachholen.
        if applied.contains(&wb.id) {
            let _ = cloud.ack_writeback_applied(&wb.id, wb.patient_id.trim()).await;
            continue;
        }
        match process_one(&mut conn, cfg, wb).await {
            Ok(Some(patnr)) => {
                applied.insert(wb.id.clone());
                if let Err(e) = cloud.ack_writeback_applied(&wb.id, &patnr).await {
                    warn!(id=%wb.id, error=%e, "Writeback angewandt, Ack fehlgeschlagen (wird nachgeholt)");
                }
            }
            Ok(None) => {
                // Patient (noch) nicht auflösbar → zurückstellen, NICHT schreiben,
                // NICHT acken. Ein späterer Zyklus versucht es erneut.
                debug!(id=%wb.id, "Writeback zurückgestellt: Patient nicht eindeutig auflösbar");
            }
            Err(e) => {
                warn!(id=%wb.id, error=%e, "Writeback fehlgeschlagen");
                let _ = cloud.ack_writeback_failed(&wb.id, &e.to_string()).await;
            }
        }
    }
    Ok(())
}

/// Verarbeitet ein Bündel. Rückgabe:
///   * `Ok(Some(patnr))` — angewandt
///   * `Ok(None)`        — zurückgestellt (Patient nicht eindeutig auflösbar)
///   * `Err(_)`          — echter Fehler
async fn process_one(
    conn: &mut Z1Connection,
    cfg: &ConnectorConfig,
    wb: &PendingWriteback,
) -> Result<Option<String>> {
    // PATNR bestimmen: geliefert → sonst Name+Geburtsdatum-Lookup.
    let patnr = {
        let given = wb.patient_id.trim();
        if !given.is_empty() {
            given.to_string()
        } else {
            match crate::z1db::resolve_patnr(conn, &wb.last_name, &wb.first_name, &wb.birth_date)
                .await?
            {
                Some(p) => p,
                None => return Ok(None), // zurückstellen (kein Neupatient-Pfad hier)
            }
        }
    };

    let mut data = PatientWriteback::from(wb);
    data.patient_id = patnr.clone();
    apply_writeback(conn, cfg, &data).await?;
    Ok(Some(patnr))
}
