//! HKP-/EBZ-Tracking über die Z1-DB (ersetzt den KIM-Watcher).
//!
//! Eine eigenständige Schleife berechnet read-only den **aktuellen Lifecycle-
//! Status jedes elektronischen Plans** (nicht nur die Entscheidung) und meldet
//! **Statuswechsel** an die Cloud — samt Meilenstein-Daten und Voll-HKP-XML fürs
//! Praxishub-Detail-Drawer. Der Status wird aus allen `EBZ`-Zeilen eines Plans
//! plus `ZPLAN`/`ZEHIT` abgeleitet (siehe `docs/Z1-DATABASE.md` §3).
//!
//! Status: `erstellt` (inkl. signiert) → `versendet` → `rueckfrage` →
//! `genehmigt`/`abgelehnt` → `eingegliedert` → `abgerechnet`. Der Terminierungs-
//! Status kommt Praxishub-seitig (Z1-Terminmodul hier ungenutzt).

use crate::cloud::{CloudClient, HkpStatusReport};
use crate::config::ConnectorConfig;
use crate::error::Result;
use crate::paths;
use crate::z1db::{self, LoopHandle, Z1Connection};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info, warn};

fn get_str(row: &tiberius::Row, col: &str) -> String {
    row.get::<&str, _>(col).unwrap_or("").trim().to_string()
}
fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Planart-Code → Label (siehe docs/Z1-DATABASE.md §3c).
fn decode_planart(code: &str) -> String {
    match code.trim() {
        "3" => "eHKP",
        "a" => "eHKP (AAV/privat)",
        "4" => "ePAR",
        "7" => "eKBR/KGL",
        "2" => "HKP/ZE",
        other => return format!("Planart {other}"),
    }
    .to_string()
}

/// Aus allen EBZ-Zeilen eines Plans aggregierte Fakten + ZPLAN/ZEHIT.
#[derive(Debug, Default, Clone)]
struct PlanFacts {
    erstell: String,
    signatur: String,
    versand: String,
    decision_date: String,
    decision_zugestellt: String,
    rueckfrage_date: String,
    eingliederung: String,
    kzveinreich: String,
    kzvabr: String,
}

/// Leitet den aktuellen Lifecycle-Status ab (reine Funktion — testbar).
/// Reihenfolge = am weitesten fortgeschrittener Zustand zuerst.
fn compute_status(f: &PlanFacts) -> &'static str {
    let set = |s: &str| !s.trim().is_empty();
    if set(&f.kzvabr) || set(&f.kzveinreich) {
        return "abgerechnet";
    }
    if set(&f.eingliederung) {
        return "eingegliedert";
    }
    // Entscheidung vs. Rückfrage: die zeitlich neuere gewinnt (Strings sind JJJJMMTT).
    let dec = f.decision_date.trim();
    let rf = f.rueckfrage_date.trim();
    if set(dec) && dec >= rf {
        return match f.decision_zugestellt.trim() {
            "1" => "genehmigt",
            "0" => "abgelehnt",
            _ => "versendet", // unklare Entscheidung → weiterhin als wartend führen
        };
    }
    if set(rf) {
        return "rueckfrage";
    }
    if set(&f.versand) {
        return "versendet";
    }
    "erstellt" // erstellt/signiert zusammengefasst
}

/// Persistenter Store: Plan-Schlüssel → zuletzt gemeldeter Status (Change-Detect).
struct StatusStore {
    map: HashMap<String, String>,
}
impl StatusStore {
    fn load() -> Self {
        let map = paths::hkp_seen_store_file()
            .ok()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { map }
    }
    /// Hat sich der Status geändert? Aktualisiert + persistiert bei Änderung.
    fn changed(&mut self, key: &str, status: &str) -> bool {
        if self.map.get(key).map(String::as_str) == Some(status) {
            return false;
        }
        self.map.insert(key.to_string(), status.to_string());
        if let Ok(p) = paths::hkp_seen_store_file() {
            let _ = serde_json::to_vec(&self.map).map(|b| std::fs::write(p, b));
        }
        true
    }
}

struct ZplanInfo {
    antragsnummer: String,
    planart: String,
    kzveinreich: String,
    kzvabr: String,
}

/// Baut die Plan-Fakten aller elektronischen Pläne (mit EBZ-Aktivität) auf.
async fn collect_plans(conn: &mut Z1Connection) -> Result<HashMap<(String, String), PlanFacts>> {
    // ZPLAN-Details je Plan.
    let zplan_rows = conn
        .rows(
            "SELECT LTRIM(RTRIM(PATNR)) AS PATNR, LTRIM(RTRIM(LFDPLAN)) AS LFDPLAN, \
                    LTRIM(RTRIM(ISNULL(ANTRAGSNUMMER,''))) AS ANTRAGSNUMMER, \
                    ISNULL(PLANART,'') AS PLANART, ISNULL(KZVEINREICHDATUM,'') AS EINREICH, \
                    ISNULL(KZVABRDATUM,'') AS ABR FROM ZPLAN",
            &[],
        )
        .await?;
    let mut zplan: HashMap<(String, String), ZplanInfo> = HashMap::new();
    for r in &zplan_rows {
        zplan.insert(
            (get_str(r, "PATNR"), get_str(r, "LFDPLAN")),
            ZplanInfo {
                antragsnummer: get_str(r, "ANTRAGSNUMMER"),
                planart: get_str(r, "PLANART"),
                kzveinreich: get_str(r, "EINREICH"),
                kzvabr: get_str(r, "ABR"),
            },
        );
    }

    // Eingliederungsdaten (ZEHIT) je Plan.
    let zehit_rows = conn
        .rows(
            "SELECT LTRIM(RTRIM(PATNR)) AS PATNR, LTRIM(RTRIM(LFDPLAN)) AS LFDPLAN, \
                    MAX(EINGLIEDERUNGSDATUM) AS EG FROM ZEHIT \
                    WHERE ISNULL(EINGLIEDERUNGSDATUM,'') <> '' \
                    GROUP BY LTRIM(RTRIM(PATNR)), LTRIM(RTRIM(LFDPLAN))",
            &[],
        )
        .await?;
    let mut eingl: HashMap<(String, String), String> = HashMap::new();
    for r in &zehit_rows {
        eingl.insert((get_str(r, "PATNR"), get_str(r, "LFDPLAN")), get_str(r, "EG"));
    }

    // EBZ-Zeilen aggregieren.
    let ebz_rows = conn
        .rows(
            "SELECT LTRIM(RTRIM(PATNR)) AS PATNR, LTRIM(RTRIM(LFDPLAN)) AS LFDPLAN, \
                    ISNULL(DOKART,'') AS DOKART, ISNULL(SIGNATURDATUM,'') AS SIG, \
                    ISNULL(VERSANDDATUM,'') AS VERS, ISNULL(ERSTELLDATUM,'') AS ERST, \
                    ISNULL(ERHALTDATUM,'') AS ERH, ISNULL(ZUGESTELLT,'') AS ZUG, \
                    ISNULL(LFDNR,'') AS LFDNR FROM EBZ",
            &[],
        )
        .await?;

    let mut plans: HashMap<(String, String), PlanFacts> = HashMap::new();
    for r in &ebz_rows {
        let key = (get_str(r, "PATNR"), get_str(r, "LFDPLAN"));
        let dokart = get_str(r, "DOKART");
        let f = plans.entry(key).or_default();
        match dokart.as_str() {
            "1" => {
                let (erst, sig, vers) = (get_str(r, "ERST"), get_str(r, "SIG"), get_str(r, "VERS"));
                if erst > f.erstell {
                    f.erstell = erst;
                }
                if sig > f.signatur {
                    f.signatur = sig;
                }
                if vers > f.versand {
                    f.versand = vers;
                }
            }
            "3" => {
                // Neueste eindeutige Entscheidung (ZUGESTELLT 0/1) nach ERHALTDATUM.
                let erh = get_str(r, "ERH");
                let zug = get_str(r, "ZUG");
                if (zug == "0" || zug == "1") && !erh.is_empty() && erh >= f.decision_date {
                    f.decision_date = erh;
                    f.decision_zugestellt = zug;
                }
            }
            "4" => {
                let erh = get_str(r, "ERH");
                if erh > f.rueckfrage_date {
                    f.rueckfrage_date = erh;
                }
            }
            _ => {}
        }
    }

    // ZPLAN/ZEHIT-Daten anreichern (nur Pläne mit EBZ-Aktivität bleiben).
    for (key, f) in plans.iter_mut() {
        if let Some(z) = zplan.get(key) {
            f.kzveinreich = z.kzveinreich.clone();
            f.kzvabr = z.kzvabr.clone();
        }
        if let Some(eg) = eingl.get(key) {
            f.eingliederung = eg.clone();
        }
    }
    Ok(plans)
}

/// Holt den Voll-HKP (EEBZ0-XML) aus `FILEPOOL` anhand der Antragsnummer.
async fn fetch_hkp_xml(conn: &mut Z1Connection, antragsnummer: &str) -> Result<Option<Vec<u8>>> {
    if antragsnummer.is_empty() {
        return Ok(None);
    }
    let pattern = format!("EEBZ0_{antragsnummer}%.xml");
    let row = conn
        .one_row(
            "SELECT TOP 1 CAST(FILEDATA AS varbinary(max)) AS DATA FROM FILEPOOL \
             WHERE FILENAME LIKE @P1",
            &[&pattern],
        )
        .await?;
    Ok(row.and_then(|r| r.get::<&[u8], _>("DATA").map(|b| b.to_vec())))
}

/// Zusatzinfos je Plan (Antragsnummer/Planart) — für den Report.
async fn plan_meta(conn: &mut Z1Connection) -> Result<HashMap<(String, String), (String, String)>> {
    let rows = conn
        .rows(
            "SELECT LTRIM(RTRIM(PATNR)) AS PATNR, LTRIM(RTRIM(LFDPLAN)) AS LFDPLAN, \
                    LTRIM(RTRIM(ISNULL(ANTRAGSNUMMER,''))) AS ANTRAGSNUMMER, \
                    ISNULL(PLANART,'') AS PLANART FROM ZPLAN",
            &[],
        )
        .await?;
    let mut m = HashMap::new();
    for r in &rows {
        m.insert(
            (get_str(r, "PATNR"), get_str(r, "LFDPLAN")),
            (get_str(r, "ANTRAGSNUMMER"), get_str(r, "PLANART")),
        );
    }
    Ok(m)
}

/// Ein Poll-Zyklus. Gibt die Anzahl gemeldeter Statuswechsel zurück.
async fn poll_once(cfg: &ConnectorConfig, cloud: &CloudClient, store: &mut StatusStore) -> Result<usize> {
    let mut conn = z1db::connect(
        &cfg.z1_db_server,
        &cfg.z1_db_database,
        &cfg.z1_db_user,
        &cfg.z1_db_password,
        cfg.z1_db_trust_cert,
    )
    .await?;

    let plans = collect_plans(&mut conn).await?;
    let meta = plan_meta(&mut conn).await?;

    let mut reported = 0usize;
    for (key, f) in &plans {
        let status = compute_status(f);
        let plan_key = format!("{}|{}", key.0, key.1);
        if !store.changed(&plan_key, status) {
            continue;
        }
        let (antragsnummer, planart_code) = meta.get(key).cloned().unwrap_or_default();
        let xml = fetch_hkp_xml(&mut conn, &antragsnummer).await.unwrap_or(None);
        let report = HkpStatusReport {
            plan_key: plan_key.clone(),
            patient_id: key.0.clone(),
            plan_no: key.1.clone(),
            antragsnummer,
            planart: decode_planart(&planart_code),
            status: status.to_string(),
            created_on: opt(&f.erstell).or_else(|| opt(&f.signatur)),
            sent_on: opt(&f.versand),
            query_on: opt(&f.rueckfrage_date),
            decided_on: opt(&f.decision_date),
            inserted_on: opt(&f.eingliederung),
            billed_on: opt(&f.kzvabr).or_else(|| opt(&f.kzveinreich)),
            ehkp_xml_b64: xml.map(|b| STANDARD.encode(b)),
        };
        match cloud.report_hkp_status(&report).await {
            Ok(()) => {
                info!(plan=%plan_key, status, "HKP-Statuswechsel gemeldet");
                reported += 1;
            }
            Err(e) => {
                // Store schon aktualisiert → bei Fehler zurücksetzen, damit Retry folgt.
                store.map.remove(&plan_key);
                warn!(plan=%plan_key, error=%e, "HKP-Status-Meldung fehlgeschlagen, Retry im nächsten Zyklus");
            }
        }
    }
    Ok(reported)
}

/// Startet den HKP-Poller als eigenständige Schleife. Läuft nur, wenn Z1-DB-Lesen
/// **und** die Cloud konfiguriert sind.
pub fn spawn(cfg: ConnectorConfig) -> LoopHandle {
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        let cloud = match CloudClient::new(&cfg) {
            Ok(c) => c,
            Err(e) => {
                warn!(error=%e, "HKP-Poller: Cloud-Client fehlgeschlagen — Schleife beendet");
                return;
            }
        };
        let mut store = StatusStore::load();
        let period = Duration::from_secs(cfg.doc_poll_seconds.max(30));
        let mut ticker = tokio::time::interval(period);
        info!(period_s = period.as_secs(), "HKP-Poller (Z1-DB, Lifecycle) gestartet");
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match poll_once(&cfg, &cloud, &mut store).await {
                        Ok(n) if n > 0 => debug!(gemeldet = n, "HKP-Poll-Zyklus"),
                        Ok(_) => {}
                        Err(e) => debug!(error=%e, "HKP-Poll-Zyklus fehlgeschlagen"),
                    }
                }
                _ = rx.changed() => {
                    if *rx.borrow() { info!("HKP-Poller gestoppt"); break; }
                }
            }
        }
    });
    LoopHandle::new(tx, join)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> PlanFacts {
        PlanFacts::default()
    }

    #[test]
    fn status_erstellt_bis_abgerechnet() {
        let mut f = facts();
        assert_eq!(compute_status(&f), "erstellt");
        f.signatur = "20260101".into(); // signiert → weiterhin "erstellt"
        assert_eq!(compute_status(&f), "erstellt");
        f.versand = "20260102".into();
        assert_eq!(compute_status(&f), "versendet");
        f.rueckfrage_date = "20260103".into();
        assert_eq!(compute_status(&f), "rueckfrage");
        f.decision_date = "20260104".into();
        f.decision_zugestellt = "1".into();
        assert_eq!(compute_status(&f), "genehmigt"); // Entscheidung neuer als Rückfrage
        f.decision_zugestellt = "0".into();
        assert_eq!(compute_status(&f), "abgelehnt");
        f.eingliederung = "20260201".into();
        assert_eq!(compute_status(&f), "eingegliedert");
        f.kzvabr = "20260301".into();
        assert_eq!(compute_status(&f), "abgerechnet");
    }

    #[test]
    fn rueckfrage_nach_entscheidung_gewinnt() {
        let mut f = facts();
        f.versand = "20260101".into();
        f.decision_date = "20260104".into();
        f.decision_zugestellt = "1".into();
        f.rueckfrage_date = "20260110".into(); // neuere Rückfrage → wieder offen
        assert_eq!(compute_status(&f), "rueckfrage");
    }
}
