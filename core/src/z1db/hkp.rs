//! HKP-/EBZ-Tracking über die Z1-DB (ersetzt den KIM-Watcher) — **fall-zentriert**.
//!
//! Eine eigenständige Schleife berechnet read-only pro **Fall** (`PATNR`+`LFDBEFUND`)
//! den aktuellen Lifecycle-Status und meldet **Statuswechsel** an die Cloud. Ein
//! Fall bündelt den GAV-Kassenplan + die AAV-Privatalternative (verknüpft über
//! `LFDBEFUND`/`LFDAPLAN`); Rückfrage-Nachreichungen sind bereits im selben Plan
//! (mehrere EBZ-Zeilen). Der Report trägt den Fall-Status, Meilenstein-Daten, das
//! Voll-HKP-XML des führenden Plans und **alle Pläne des Falls samt EBZ-Verlauf**
//! fürs Detail-Drawer. Siehe `docs/Z1-DATABASE.md` §3.
//!
//! Status: `erstellt` (inkl. signiert) → `versendet` → `rueckfrage` →
//! `genehmigt`/`abgelehnt` → `eingegliedert` → `abgerechnet`. Sonderfall
//! **`abgelaufen`**: genehmigt, nicht eingegliedert, in Z1 deaktiviert
//! (`DEAKTIVIERTDATUM`) **oder** über Gültigkeit (Genehmigung + 6 Monate) hinaus →
//! verlorener Umsatz. Terminierungs-Status kommt Praxishub-seitig.

use crate::cloud::{CloudClient, HkpCaseReport, HkpPlanEntry, HkpSubmission};
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
fn lfd_num(s: &str) -> i64 {
    s.trim().parse().unwrap_or(0)
}

/// Planart-Code → Label (siehe docs/Z1-DATABASE.md §3c).
fn decode_planart(code: &str) -> String {
    match code.trim() {
        "3" | "a" => "eHKP",
        "4" => "ePAR",
        "7" => "eKBR/KGL",
        "2" => "HKP/ZE",
        other => return format!("Planart {other}"),
    }
    .to_string()
}

/// Gültigkeit einer HKP-Genehmigung in Monaten (ZE-Standard; ggf. je Planart tunen).
const VALIDITY_MONTHS: u32 = 6;

fn parse_ymd(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y%m%d").ok()
}
fn add_months_ymd(s: &str, months: u32) -> Option<String> {
    parse_ymd(s)
        .and_then(|d| d.checked_add_months(chrono::Months::new(months)))
        .map(|d| d.format("%Y%m%d").to_string())
}

/// Aus allen EBZ-Zeilen eines Plans aggregierte Fakten.
#[derive(Debug, Default, Clone)]
struct PlanFacts {
    erstell: String,
    signatur: String,
    versand: String,
    decision_date: String,
    decision_zugestellt: String,
    rueckfrage_date: String,
    eingliederung: String,
    kzvabr: String,
    deaktiviert: String,
}

/// Leitet den Lifecycle-Status ab (reine Funktion — testbar). `expiry_cutoff` =
/// `JJJJMMTT` von (heute − Gültigkeit); Genehmigungen davor gelten als abgelaufen.
fn compute_status(f: &PlanFacts, expiry_cutoff: &str) -> &'static str {
    let set = |s: &str| !s.trim().is_empty();
    if set(&f.kzvabr) {
        return "abgerechnet";
    }
    if set(&f.eingliederung) {
        return "eingegliedert";
    }
    let dec = f.decision_date.trim();
    let rf = f.rueckfrage_date.trim();
    if set(dec) && dec >= rf {
        return match f.decision_zugestellt.trim() {
            "1" => {
                let expired = !expiry_cutoff.is_empty() && dec < expiry_cutoff;
                if set(&f.deaktiviert) || expired {
                    "abgelaufen"
                } else {
                    "genehmigt"
                }
            }
            "0" => "abgelehnt",
            _ => "versendet",
        };
    }
    if set(rf) {
        return "rueckfrage";
    }
    if set(&f.versand) {
        return "versendet";
    }
    "erstellt"
}

/// ZPLAN-Zeile (ein Plan), für Fallbildung + Drawer.
#[derive(Debug, Clone, Default)]
struct PlanRow {
    lfdbefund: String,
    planart: String,
    planungsdatum: String,
    antragsnummer: String,
    kzvabr: String,
    deaktiviert: String,
}

/// Persistenter Store: Fall-Schlüssel → zuletzt gemeldeter Status (Change-Detect).
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

type PlanKey = (String, String); // (PATNR, LFDPLAN)

/// Lädt alle Pläne (ZPLAN), EBZ-Fakten je Plan, EBZ-Verlauf je Plan und Eingliederung.
async fn load_all(
    conn: &mut Z1Connection,
) -> Result<(
    HashMap<PlanKey, PlanRow>,
    HashMap<PlanKey, PlanFacts>,
    HashMap<PlanKey, Vec<HkpSubmission>>,
)> {
    // ZPLAN (alle Pläne).
    let zrows = conn
        .rows(
            "SELECT LTRIM(RTRIM(PATNR)) AS PATNR, LTRIM(RTRIM(LFDPLAN)) AS LFDPLAN, \
                    LTRIM(RTRIM(ISNULL(LFDBEFUND,''))) AS LFDBEFUND, ISNULL(PLANART,'') AS PLANART, \
                    ISNULL(PLANUNGSDATUM,'') AS PLNG, LTRIM(RTRIM(ISNULL(ANTRAGSNUMMER,''))) AS ANTRAG, \
                    ISNULL(KZVABRDATUM,'') AS ABR, ISNULL(DEAKTIVIERTDATUM,'') AS DEAKT FROM ZPLAN",
            &[],
        )
        .await?;
    let mut plans: HashMap<PlanKey, PlanRow> = HashMap::new();
    for r in &zrows {
        plans.insert(
            (get_str(r, "PATNR"), get_str(r, "LFDPLAN")),
            PlanRow {
                lfdbefund: get_str(r, "LFDBEFUND"),
                planart: get_str(r, "PLANART"),
                planungsdatum: get_str(r, "PLNG"),
                antragsnummer: get_str(r, "ANTRAG"),
                kzvabr: get_str(r, "ABR"),
                deaktiviert: get_str(r, "DEAKT"),
            },
        );
    }

    // Eingliederung (ZEHIT).
    let hrows = conn
        .rows(
            "SELECT LTRIM(RTRIM(PATNR)) AS PATNR, LTRIM(RTRIM(LFDPLAN)) AS LFDPLAN, \
                    MAX(EINGLIEDERUNGSDATUM) AS EG FROM ZEHIT WHERE ISNULL(EINGLIEDERUNGSDATUM,'')<>'' \
                    GROUP BY LTRIM(RTRIM(PATNR)), LTRIM(RTRIM(LFDPLAN))",
            &[],
        )
        .await?;
    let mut eingl: HashMap<PlanKey, String> = HashMap::new();
    for r in &hrows {
        eingl.insert((get_str(r, "PATNR"), get_str(r, "LFDPLAN")), get_str(r, "EG"));
    }

    // EBZ-Zeilen → Fakten + Verlauf je Plan.
    let erows = conn
        .rows(
            "SELECT LTRIM(RTRIM(PATNR)) AS PATNR, LTRIM(RTRIM(LFDPLAN)) AS LFDPLAN, \
                    ISNULL(DOKART,'') AS DOKART, ISNULL(SIGNATURDATUM,'') AS SIG, \
                    ISNULL(VERSANDDATUM,'') AS VERS, ISNULL(ERSTELLDATUM,'') AS ERST, \
                    ISNULL(ERHALTDATUM,'') AS ERH, ISNULL(ZUGESTELLT,'') AS ZUG FROM EBZ",
            &[],
        )
        .await?;
    let mut facts: HashMap<PlanKey, PlanFacts> = HashMap::new();
    let mut subs: HashMap<PlanKey, Vec<HkpSubmission>> = HashMap::new();
    for r in &erows {
        let key = (get_str(r, "PATNR"), get_str(r, "LFDPLAN"));
        let dokart = get_str(r, "DOKART");
        let (sig, vers, erst, erh, zug) = (
            get_str(r, "SIG"),
            get_str(r, "VERS"),
            get_str(r, "ERST"),
            get_str(r, "ERH"),
            get_str(r, "ZUG"),
        );
        let f = facts.entry(key.clone()).or_default();
        let list = subs.entry(key).or_default();
        match dokart.as_str() {
            "1" => {
                if erst > f.erstell {
                    f.erstell = erst.clone();
                }
                if sig > f.signatur {
                    f.signatur = sig.clone();
                }
                if vers > f.versand {
                    f.versand = vers.clone();
                }
                let date = [&vers, &sig, &erst].into_iter().find(|s| !s.is_empty()).cloned().unwrap_or_default();
                if !date.is_empty() {
                    list.push(HkpSubmission { kind: "antrag".into(), date, result: None });
                }
            }
            "2" => {
                let date = [&vers, &erst].into_iter().find(|s| !s.is_empty()).cloned().unwrap_or_default();
                if !date.is_empty() {
                    list.push(HkpSubmission { kind: "nachreichung".into(), date, result: None });
                }
            }
            "3" => {
                if (zug == "0" || zug == "1") && !erh.is_empty() && erh >= f.decision_date {
                    f.decision_date = erh.clone();
                    f.decision_zugestellt = zug.clone();
                }
                if !erh.is_empty() {
                    let result = match zug.as_str() {
                        "1" => Some("genehmigt".to_string()),
                        "0" => Some("abgelehnt".to_string()),
                        _ => None,
                    };
                    list.push(HkpSubmission { kind: "antwort".into(), date: erh, result });
                }
            }
            "4" => {
                if erh > f.rueckfrage_date {
                    f.rueckfrage_date = erh.clone();
                }
                if !erh.is_empty() {
                    list.push(HkpSubmission { kind: "rueckfrage".into(), date: erh, result: None });
                }
            }
            _ => {}
        }
    }

    // ZPLAN/ZEHIT in die Fakten spiegeln.
    for (key, f) in facts.iter_mut() {
        if let Some(z) = plans.get(key) {
            f.kzvabr = z.kzvabr.clone();
            f.deaktiviert = z.deaktiviert.clone();
        }
        if let Some(eg) = eingl.get(key) {
            f.eingliederung = eg.clone();
        }
    }
    // Verlauf je Plan zeitlich sortieren.
    for list in subs.values_mut() {
        list.sort_by(|a, b| a.date.cmp(&b.date));
    }
    Ok((plans, facts, subs))
}

/// Holt den Voll-HKP (EEBZ0-XML) aus `FILEPOOL` anhand der Antragsnummer.
async fn fetch_hkp_xml(conn: &mut Z1Connection, antragsnummer: &str) -> Result<Option<Vec<u8>>> {
    if antragsnummer.is_empty() {
        return Ok(None);
    }
    let pattern = format!("EEBZ0_{antragsnummer}%.xml");
    let row = conn
        .one_row(
            "SELECT TOP 1 CAST(FILEDATA AS varbinary(max)) AS DATA FROM FILEPOOL WHERE FILENAME LIKE @P1",
            &[&pattern],
        )
        .await?;
    Ok(row.and_then(|r| r.get::<&[u8], _>("DATA").map(|b| b.to_vec())))
}

fn variant_of(planart: &str) -> &'static str {
    if planart.trim() == "a" {
        "AAV"
    } else {
        "GAV"
    }
}

/// Ein Poll-Zyklus. Gibt die Anzahl gemeldeter Fall-Statuswechsel zurück.
async fn poll_once(cfg: &ConnectorConfig, cloud: &CloudClient, store: &mut StatusStore) -> Result<usize> {
    let mut conn = z1db::connect(
        &cfg.z1_db_server,
        &cfg.z1_db_database,
        &cfg.z1_db_user,
        &cfg.z1_db_password,
        cfg.z1_db_trust_cert,
    )
    .await?;

    let (plans, facts, subs) = load_all(&mut conn).await?;

    let expiry_cutoff = chrono::Local::now()
        .date_naive()
        .checked_sub_months(chrono::Months::new(VALIDITY_MONTHS))
        .map(|d| d.format("%Y%m%d").to_string())
        .unwrap_or_default();

    // Pläne zu Fällen (PATNR|LFDBEFUND) gruppieren; nur Fälle mit LFDBEFUND.
    let mut cases: HashMap<String, Vec<PlanKey>> = HashMap::new();
    for (key, row) in &plans {
        if row.lfdbefund.is_empty() {
            continue;
        }
        let case_key = format!("{}|{}", key.0, row.lfdbefund);
        cases.entry(case_key).or_default().push(key.clone());
    }

    let mut reported = 0usize;
    for (case_key, mut members) in cases {
        // Nur Fälle mit EBZ-Aktivität (elektronisch) tracken.
        if !members.iter().any(|k| facts.contains_key(k)) {
            continue;
        }
        // Führender Plan = neuester GAV-Plan mit EBZ (max Planungsdatum, dann LFDPLAN).
        members.sort_by(|a, b| {
            let (pa, pb) = (&plans[a], &plans[b]);
            pb.planungsdatum
                .cmp(&pa.planungsdatum)
                .then(lfd_num(&b.1).cmp(&lfd_num(&a.1)))
        });
        let Some(primary) = members
            .iter()
            .find(|k| facts.contains_key(*k) && variant_of(&plans[*k].planart) == "GAV")
            .or_else(|| members.iter().find(|k| facts.contains_key(*k)))
            .cloned()
        else {
            continue;
        };

        let pfacts = facts.get(&primary).cloned().unwrap_or_default();
        let status = compute_status(&pfacts, &expiry_cutoff);
        if !store.changed(&case_key, status) {
            continue;
        }

        let prow = &plans[&primary];
        let xml = fetch_hkp_xml(&mut conn, &prow.antragsnummer).await.unwrap_or(None);

        // Alle Pläne des Falls fürs Drawer.
        let mut entries: Vec<HkpPlanEntry> = members
            .iter()
            .map(|k| {
                let row = &plans[k];
                let is_primary = *k == primary;
                let pstatus = if variant_of(&row.planart) == "AAV" {
                    "privat".to_string()
                } else if let Some(ff) = facts.get(k) {
                    compute_status(ff, &expiry_cutoff).to_string()
                } else {
                    "erstellt".to_string()
                };
                HkpPlanEntry {
                    plan_no: k.1.clone(),
                    variant: variant_of(&row.planart).to_string(),
                    is_primary,
                    planart: decode_planart(&row.planart),
                    antragsnummer: row.antragsnummer.clone(),
                    status: pstatus,
                    planned_on: opt(&row.planungsdatum),
                    submissions: subs.get(k).cloned().unwrap_or_default(),
                }
            })
            .collect();
        entries.sort_by(|a, b| a.planned_on.cmp(&b.planned_on).then(a.plan_no.cmp(&b.plan_no)));

        let report = HkpCaseReport {
            case_key: case_key.clone(),
            patient_id: primary.0.clone(),
            befund_no: prow.lfdbefund.clone(),
            planart: decode_planart(&prow.planart),
            status: status.to_string(),
            created_on: opt(&pfacts.erstell).or_else(|| opt(&pfacts.signatur)),
            sent_on: opt(&pfacts.versand),
            query_on: opt(&pfacts.rueckfrage_date),
            decided_on: opt(&pfacts.decision_date),
            inserted_on: opt(&pfacts.eingliederung),
            billed_on: opt(&pfacts.kzvabr),
            valid_until: add_months_ymd(&pfacts.decision_date, VALIDITY_MONTHS),
            ehkp_xml_b64: xml.map(|b| STANDARD.encode(b)),
            plans: entries,
        };
        match cloud.report_hkp_case(&report).await {
            Ok(()) => {
                info!(case=%case_key, status, "HKP-Fall-Statuswechsel gemeldet");
                reported += 1;
            }
            Err(e) => {
                store.map.remove(&case_key); // Retry im nächsten Zyklus
                warn!(case=%case_key, error=%e, "HKP-Fall-Meldung fehlgeschlagen");
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
        info!(period_s = period.as_secs(), "HKP-Poller (Z1-DB, fall-zentriert) gestartet");
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
    const CUTOFF: &str = "20250701";
    fn facts() -> PlanFacts {
        PlanFacts::default()
    }

    #[test]
    fn status_erstellt_bis_abgerechnet() {
        let mut f = facts();
        assert_eq!(compute_status(&f, CUTOFF), "erstellt");
        f.signatur = "20260101".into();
        assert_eq!(compute_status(&f, CUTOFF), "erstellt");
        f.versand = "20260102".into();
        assert_eq!(compute_status(&f, CUTOFF), "versendet");
        f.rueckfrage_date = "20260103".into();
        assert_eq!(compute_status(&f, CUTOFF), "rueckfrage");
        f.decision_date = "20260104".into();
        f.decision_zugestellt = "1".into();
        assert_eq!(compute_status(&f, CUTOFF), "genehmigt");
        f.decision_zugestellt = "0".into();
        assert_eq!(compute_status(&f, CUTOFF), "abgelehnt");
        f.decision_zugestellt = "1".into();
        f.eingliederung = "20260201".into();
        assert_eq!(compute_status(&f, CUTOFF), "eingegliedert");
        f.kzvabr = "20260301".into();
        assert_eq!(compute_status(&f, CUTOFF), "abgerechnet");
    }

    #[test]
    fn genehmigt_ueber_frist_oder_deaktiviert_ist_abgelaufen() {
        let mut f = facts();
        f.decision_date = "20250115".into();
        f.decision_zugestellt = "1".into();
        assert_eq!(compute_status(&f, CUTOFF), "abgelaufen"); // über Frist
        let mut g = facts();
        g.decision_date = "20260601".into();
        g.decision_zugestellt = "1".into();
        g.deaktiviert = "20260701".into();
        assert_eq!(compute_status(&g, CUTOFF), "abgelaufen"); // deaktiviert
    }

    #[test]
    fn rueckfrage_nach_entscheidung_gewinnt() {
        let mut f = facts();
        f.decision_date = "20260104".into();
        f.decision_zugestellt = "1".into();
        f.rueckfrage_date = "20260110".into();
        assert_eq!(compute_status(&f, CUTOFF), "rueckfrage");
    }

    #[test]
    fn variante_gav_aav() {
        assert_eq!(variant_of("3"), "GAV");
        assert_eq!(variant_of("a"), "AAV");
        assert_eq!(variant_of("4"), "GAV");
    }
}
