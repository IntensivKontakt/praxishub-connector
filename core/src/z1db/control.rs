//! Praxis-Steuerung: **nächtlicher Aggregat-Sync** aus der Z1-DB → Cloud
//! (`POST /api/v1/connector/z1/control-report`).
//!
//! Der Connector aggregiert **SQL-seitig** (BEH hat ~1,46 Mio. Zeilen → kleine
//! Ergebnismengen) und liefert vier Report-Teile: `revenue` (Honorar je Monat ×
//! Art × Behandler), `payments` (Zahlungseingänge je Monat × Art), `ar_aging`
//! (offene Forderungen nach Alters-Buckets) und `open_services` (erbrachte
//! Leistungen ohne Abrechnungsbezug).
//!
//! **Wichtig:** Außer den in `docs/Z1-DATABASE.md` §3 belegten Spalten (z. B.
//! `PATNR`, `LFDPLAN`) sind die echten Spaltennamen der Abrechnungstabellen
//! (`BEH`/`LBLOCKENTRY`/`BILL`/`FAKT`/`KONTO`/`CASH`) **unbekannt**. Deshalb:
//!   1. **Schema-Discovery** (INFORMATION_SCHEMA) läuft bei jedem Sync mit und
//!      wird als `sync.schema` mitgeschickt — die Cloud speichert sie.
//!   2. Eine [`ColumnMap`] enthält Default-**Vermutungen** und ist per Config
//!      (`z1_control_column_map`) überschreibbar → Zuordnung wird am Piloten
//!      ohne Neubau finalisiert.
//!   3. **Gate je Report-Teil:** fehlt auch nur eine benötigte Spalte, wird der
//!      Teil AUSGELASSEN und unter `sync.pending_mappings` gemeldet (fehlende +
//!      tatsächlich vorhandene Spalten). Es wird **nie geraten**, es werden
//!      **nie erfundene Zahlen** geliefert.
//!
//! Ablauf: [`spawn`] tickt stündlich, führt den Sync aber nur **einmal pro Tag**
//! (persistierter Marker) am oder nach der frühesten Stunde `z1_control_hour` aus —
//! war der PC in der Nacht aus, wird der Lauf am Morgen nachgeholt (Anacron-Prinzip);
//! Timeout 300 s pro Zyklus (Muster `hkp.rs`).

use crate::cloud::{
    ArAgingRow, CloudClient, ControlReport, ControlSync, OpenServicesRow, PaymentRow, RevenueRow,
};
use crate::config::ConnectorConfig;
use crate::error::Result;
use crate::paths;
use crate::z1db::{self, LoopHandle, Z1Connection};
use chrono::Timelike;
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::{info, warn};

/// Zeitlimit für einen kompletten Sync-Zyklus (Discovery + Aggregate + Push).
const CYCLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Tabellen, deren Spalten per Discovery erhoben und mitgeschickt werden.
const DISCOVERY_TABLES: [&str; 7] =
    ["BEH", "LBLOCK", "LBLOCKENTRY", "BILL", "FAKT", "KONTO", "CASH"];

fn get_str(row: &tiberius::Row, col: &str) -> String {
    row.get::<&str, _>(col).unwrap_or("").trim().to_string()
}

/// Identifier-Härtung fürs Interpolieren in SQL (nur `[A-Za-z0-9_]`). Das Gate
/// stellt ohnehin sicher, dass der Name real in der DB existiert — dies ist die
/// zweite Verteidigungslinie gegen kaputte Config-Overrides.
fn ident(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// SQL-Ausdruck, der einen varchar-Betrag robust nach `float` wandelt — sowohl
/// deutsche Komma-Notation (`1.234,56`) als auch Punkt-Notation (`1234.56`).
fn sql_amount(expr: &str) -> String {
    format!(
        "ISNULL(TRY_CAST(CASE WHEN {e} LIKE '%,%' \
             THEN REPLACE(REPLACE({e}, '.', ''), ',', '.') ELSE {e} END AS float), 0)",
        e = expr
    )
}

/// Rust-Pendant zu [`sql_amount`] (dokumentiert + testet die Parsing-Semantik;
/// das produktive Parsing passiert SQL-seitig in [`sql_amount`]).
#[cfg_attr(not(test), allow(dead_code))]
fn parse_amount(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    let cleaned = if t.contains(',') {
        t.replace('.', "").replace(',', ".")
    } else {
        t.to_string()
    };
    cleaned.parse().unwrap_or(0.0)
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// `JJJJMM…` → Monats-Periode `"JJJJ-MM-01"` (None bei Müll).
fn period_from_ym(ym: &str) -> Option<String> {
    let t = ym.trim();
    if t.len() < 6 || !t[..6].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let month: u32 = t[4..6].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some(format!("{}-{}-01", &t[..4], &t[4..6]))
}

/// `JJJJMMTT` → ISO-Datum `"JJJJ-MM-TT"` (None bei Müll).
fn ymd_to_iso(s: &str) -> Option<String> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y%m%d")
        .ok()
        .map(|d| d.format("%Y-%m-%d").to_string())
}

/// Alters-Bucket einer offenen Forderung (Tage seit Rechnungsdatum).
fn bucket_for(days: i64) -> &'static str {
    if days <= 30 {
        "0-30"
    } else if days <= 60 {
        "31-60"
    } else if days <= 90 {
        "61-90"
    } else {
        "90+"
    }
}

/// Rohwert der Leistungs-Art → Cloud-Vokabular `bema|goz|privat`; unbekannte
/// Werte werden **unverändert** (kleingeschrieben) durchgereicht, damit die
/// Cloud die echten Z1-Werte sieht und das Mapping am Piloten finalisiert wird.
fn map_art(raw: &str) -> String {
    let l = raw.trim().to_lowercase();
    if l.contains("bema") {
        "bema".into()
    } else if l.contains("goz") {
        "goz".into()
    } else if l.contains("priv") {
        "privat".into()
    } else if l.is_empty() {
        "unbekannt".into()
    } else {
        l
    }
}

// ── ColumnMap ────────────────────────────────────────────────────────────────

/// Vermutete Spaltennamen der Abrechnungstabellen. **Nur `PATNR` ist belegt**
/// (docs/Z1-DATABASE.md §2/§3); alles andere sind Default-Vermutungen, die per
/// `z1_control_column_map` (JSON-Objekt, Feldname → Spaltenname) überschrieben
/// werden. Ein Teil-Report läuft erst, wenn ALLE seine Spalten laut Discovery
/// existieren (siehe Gate in [`sync_once`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ColumnMap {
    // BEH (Leistungshistorie)
    pub beh_patnr: String,
    /// Leistungsdatum (`JJJJMMTT`).
    pub beh_datum: String,
    /// Leistungsart (BEMA/GOZ/privat …).
    pub beh_art: String,
    /// Leistungsziffer (derzeit nicht in den Aggregaten benutzt; Reserve für
    /// die spätere `gruppe`-Ableitung).
    pub beh_ziffer: String,
    /// Behandler-Kürzel.
    pub beh_behandler: String,
    /// Verknüpfung zum Leistungsblock (→ `LBLOCKENTRY`).
    pub beh_block: String,
    /// Abrechnungsbezug (→ `BILL`); leer = noch nicht abgerechnet.
    pub beh_bill: String,
    // LBLOCKENTRY (Einzelbeträge je Block)
    pub lblockentry_block: String,
    pub lblockentry_betrag: String,
    // KONTO (Zahlungseingänge, rechnungsbezogen)
    pub konto_datum: String,
    pub konto_betrag: String,
    pub konto_zahlart: String,
    /// Rechnungs-Verknüpfung (→ `FAKT`), für `ar_aging` (FAKT − KONTO).
    pub konto_rechnr: String,
    // CASH (Barzahlungen/Kasse)
    pub cash_datum: String,
    pub cash_betrag: String,
    pub cash_zahlart: String,
    // FAKT (Rechnungen)
    pub fakt_rechnr: String,
    pub fakt_datum: String,
    pub fakt_betrag: String,
    /// Offen-Kennzeichen (Reserve; `ar_aging` rechnet FAKT − KONTO).
    pub fakt_offen: String,
    // BILL (Abrechnungen)
    pub bill_key: String,
}

impl Default for ColumnMap {
    fn default() -> Self {
        Self {
            beh_patnr: "PATNR".into(), // belegt (§2)
            beh_datum: "DATUM".into(),
            beh_art: "LEISTUNGSART".into(),
            beh_ziffer: "LEISTUNG".into(),
            beh_behandler: "BEHANDLER".into(),
            beh_block: "LFDLBLOCK".into(),
            beh_bill: "LFDBILL".into(),
            lblockentry_block: "LFDLBLOCK".into(),
            lblockentry_betrag: "EINZELBETRAG".into(),
            konto_datum: "DATUM".into(),
            konto_betrag: "BETRAG".into(),
            konto_zahlart: "ZAHLART".into(),
            konto_rechnr: "RECHNR".into(),
            cash_datum: "DATUM".into(),
            cash_betrag: "BETRAG".into(),
            cash_zahlart: "ZAHLART".into(),
            fakt_rechnr: "RECHNR".into(),
            fakt_datum: "RECHNUNGSDATUM".into(),
            fakt_betrag: "BETRAG".into(),
            fakt_offen: "OFFEN".into(),
            bill_key: "LFDBILL".into(),
        }
    }
}

impl ColumnMap {
    /// Defaults + Config-Overrides (`z1_control_column_map`, nur String-Werte).
    pub fn resolved(overrides: Option<&serde_json::Value>) -> Self {
        let mut base = match serde_json::to_value(Self::default()) {
            Ok(v) => v,
            Err(_) => return Self::default(),
        };
        if let (Some(obj), Some(serde_json::Value::Object(over))) = (base.as_object_mut(), overrides)
        {
            for (k, v) in over {
                if v.is_string() {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        serde_json::from_value(base).unwrap_or_default()
    }
}

// ── Schema-Discovery + Gate ──────────────────────────────────────────────────

/// Ergebnis der INFORMATION_SCHEMA-Discovery: Tabelle (GROSS) → Spalten in
/// DB-Reihenfolge.
struct Discovery {
    tables: BTreeMap<String, Vec<String>>,
}

impl Discovery {
    fn from_pairs<I: IntoIterator<Item = (String, String)>>(pairs: I) -> Self {
        let mut tables: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (t, c) in pairs {
            tables.entry(t.to_uppercase()).or_default().push(c);
        }
        Self { tables }
    }

    fn has(&self, table: &str, col: &str) -> bool {
        self.tables
            .get(&table.to_uppercase())
            .is_some_and(|cols| cols.iter().any(|c| c.eq_ignore_ascii_case(col)))
    }

    /// Fehlende Spalten einer Anforderungsliste, als `TABELLE.SPALTE`.
    fn missing(&self, required: &[(&str, &str)]) -> Vec<String> {
        required
            .iter()
            .filter(|(t, c)| !self.has(t, c))
            .map(|(t, c)| format!("{t}.{}", c.to_uppercase()))
            .collect()
    }

    /// Tatsächlich vorhandene Spalten der beteiligten Tabellen (`TABELLE.SPALTE`).
    fn available(&self, tables: &[&str]) -> Vec<String> {
        tables
            .iter()
            .flat_map(|t| {
                self.tables
                    .get(&t.to_uppercase())
                    .into_iter()
                    .flatten()
                    .map(move |c| format!("{t}.{c}"))
            })
            .collect()
    }

    /// `{tabelle: [spalten…]}` für `sync.schema`.
    fn schema_json(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.tables
                .iter()
                .map(|(t, cols)| {
                    (
                        t.clone(),
                        serde_json::Value::Array(
                            cols.iter().cloned().map(serde_json::Value::String).collect(),
                        ),
                    )
                })
                .collect(),
        )
    }
}

async fn discover_schema(conn: &mut Z1Connection) -> Result<Discovery> {
    let in_list = DISCOVERY_TABLES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT TABLE_NAME, COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS \
         WHERE UPPER(TABLE_NAME) IN ({in_list}) ORDER BY TABLE_NAME, ORDINAL_POSITION"
    );
    let rows = conn.rows(&sql, &[]).await?;
    Ok(Discovery::from_pairs(rows.iter().map(|r| {
        (get_str(r, "TABLE_NAME"), get_str(r, "COLUMN_NAME"))
    })))
}

fn pending_entry(missing: Vec<String>, disc: &Discovery, tables: &[&str]) -> serde_json::Value {
    serde_json::json!({ "missing": missing, "available": disc.available(tables) })
}

// ── Aggregat-Queries (laufen nur bei grünem Gate) ────────────────────────────

async fn count_beh_since(conn: &mut Z1Connection, m: &ColumnMap, cutoff: &str) -> Result<i64> {
    let sql = format!(
        "SELECT COUNT_BIG(*) FROM BEH WHERE ISNULL([{d}],'') >= @P1",
        d = ident(&m.beh_datum)
    );
    let row = conn.one_row(&sql, &[&cutoff]).await?;
    Ok(row.and_then(|r| r.get::<i64, _>(0)).unwrap_or(0))
}

/// Honorar je Monat × Art × Behandler aus `BEH ⋈ LBLOCKENTRY`.
async fn query_revenue(
    conn: &mut Z1Connection,
    m: &ColumnMap,
    cutoff: &str,
) -> Result<Vec<RevenueRow>> {
    let amt = sql_amount(&format!("e.[{}]", ident(&m.lblockentry_betrag)));
    let sql = format!(
        "SELECT SUBSTRING(ISNULL(b.[{d}],''),1,6) AS YM, \
                LTRIM(RTRIM(ISNULL(b.[{a}],''))) AS ART, \
                LTRIM(RTRIM(ISNULL(b.[{beh}],''))) AS BEHANDLER, \
                SUM({amt}) AS HONORAR, \
                CAST(COUNT(*) AS int) AS N_LEIST, \
                CAST(COUNT(DISTINCT LTRIM(RTRIM(b.[{p}]))) AS int) AS N_FAELLE \
         FROM BEH b \
         JOIN LBLOCKENTRY e ON LTRIM(RTRIM(e.[{eb}])) = LTRIM(RTRIM(b.[{bb}])) \
         WHERE ISNULL(b.[{d}],'') >= @P1 \
         GROUP BY SUBSTRING(ISNULL(b.[{d}],''),1,6), \
                  LTRIM(RTRIM(ISNULL(b.[{a}],''))), \
                  LTRIM(RTRIM(ISNULL(b.[{beh}],'')))",
        d = ident(&m.beh_datum),
        a = ident(&m.beh_art),
        beh = ident(&m.beh_behandler),
        p = ident(&m.beh_patnr),
        eb = ident(&m.lblockentry_block),
        bb = ident(&m.beh_block),
    );
    let rows = conn.rows(&sql, &[&cutoff]).await?;
    let mut out = Vec::new();
    for r in &rows {
        let Some(period) = period_from_ym(&get_str(r, "YM")) else {
            continue; // Müll-Datum → nicht erfinden, auslassen
        };
        out.push(RevenueRow {
            period,
            art: map_art(&get_str(r, "ART")),
            gruppe: None,   // keine belastbare Gruppen-Spalte bekannt → null
            behandler: get_str(r, "BEHANDLER"),
            standort: None, // Z1-Einzelstandort; Spalte unbekannt → null
            honorar: round2(r.get::<f64, _>("HONORAR").unwrap_or(0.0)),
            eigenlabor: None, // Labor-Spalten unbekannt → null (nicht 0 erfinden)
            fremdlabor: None,
            n_leistungen: i64::from(r.get::<i32, _>("N_LEIST").unwrap_or(0)),
            n_faelle: i64::from(r.get::<i32, _>("N_FAELLE").unwrap_or(0)),
        });
    }
    out.sort_by(|a, b| {
        (&a.period, &a.art, &a.behandler).cmp(&(&b.period, &b.art, &b.behandler))
    });
    Ok(out)
}

/// Zahlungseingänge je Monat × Zahlart aus einer Quelle (`KONTO` oder `CASH`).
async fn query_payments_source(
    conn: &mut Z1Connection,
    table: &str,
    datum: &str,
    betrag: &str,
    zahlart: &str,
    cutoff: &str,
    acc: &mut BTreeMap<(String, String), (f64, i64)>,
) -> Result<()> {
    let amt = sql_amount(&format!("[{}]", ident(betrag)));
    let sql = format!(
        "SELECT SUBSTRING(ISNULL([{d}],''),1,6) AS YM, \
                LTRIM(RTRIM(ISNULL([{z}],''))) AS ART, \
                SUM({amt}) AS SUMME, CAST(COUNT(*) AS int) AS N \
         FROM {table} WHERE ISNULL([{d}],'') >= @P1 \
         GROUP BY SUBSTRING(ISNULL([{d}],''),1,6), LTRIM(RTRIM(ISNULL([{z}],'')))",
        d = ident(datum),
        z = ident(zahlart),
    );
    let rows = conn.rows(&sql, &[&cutoff]).await?;
    for r in &rows {
        let Some(period) = period_from_ym(&get_str(r, "YM")) else {
            continue;
        };
        let art = map_art(&get_str(r, "ART"));
        let e = acc.entry((period, art)).or_insert((0.0, 0));
        e.0 += r.get::<f64, _>("SUMME").unwrap_or(0.0);
        e.1 += i64::from(r.get::<i32, _>("N").unwrap_or(0));
    }
    Ok(())
}

/// Offene Forderungen (FAKT − KONTO) je Rechnung; Bucket-Zuordnung in Rust.
async fn query_ar_aging(
    conn: &mut Z1Connection,
    m: &ColumnMap,
    snapshot_date: &str,
    today: chrono::NaiveDate,
) -> Result<Vec<ArAgingRow>> {
    let amt_f = sql_amount(&format!("[{}]", ident(&m.fakt_betrag)));
    let amt_k = sql_amount(&format!("[{}]", ident(&m.konto_betrag)));
    let sql = format!(
        "SELECT f.DAT AS DAT, f.SUMME - ISNULL(k.SUMME, 0) AS OFFEN \
         FROM (SELECT LTRIM(RTRIM([{fr}])) AS RNR, MAX(ISNULL([{fd}],'')) AS DAT, \
                      SUM({amt_f}) AS SUMME FROM FAKT GROUP BY LTRIM(RTRIM([{fr}]))) f \
         LEFT JOIN (SELECT LTRIM(RTRIM([{kr}])) AS RNR, SUM({amt_k}) AS SUMME \
                    FROM KONTO GROUP BY LTRIM(RTRIM([{kr}]))) k ON k.RNR = f.RNR \
         WHERE f.SUMME - ISNULL(k.SUMME, 0) > 0.005",
        fr = ident(&m.fakt_rechnr),
        fd = ident(&m.fakt_datum),
        kr = ident(&m.konto_rechnr),
    );
    let rows = conn.rows(&sql, &[]).await?;
    let mut agg: BTreeMap<&'static str, (f64, i64)> = BTreeMap::new();
    for r in &rows {
        let offen = r.get::<f64, _>("OFFEN").unwrap_or(0.0);
        let bucket = chrono::NaiveDate::parse_from_str(get_str(r, "DAT").as_str(), "%Y%m%d")
            .ok()
            .map(|d| bucket_for((today - d).num_days()))
            .unwrap_or("unbekannt"); // Datum unlesbar → ehrlich ausweisen
        let e = agg.entry(bucket).or_insert((0.0, 0));
        e.0 += offen;
        e.1 += 1;
    }
    Ok(agg
        .into_iter()
        .map(|(bucket, (offen, n))| ArAgingRow {
            snapshot_date: snapshot_date.to_string(),
            bucket: bucket.to_string(),
            offen: round2(offen),
            n,
        })
        .collect())
}

/// Erbrachte Leistungen ohne Abrechnungsbezug (BEH ohne BILL), je Behandler.
async fn query_open_services(
    conn: &mut Z1Connection,
    m: &ColumnMap,
    cutoff: &str,
    snapshot_date: &str,
) -> Result<Vec<OpenServicesRow>> {
    let amt = sql_amount(&format!("e.[{}]", ident(&m.lblockentry_betrag)));
    let sql = format!(
        "SELECT LTRIM(RTRIM(ISNULL(b.[{beh}],''))) AS BEHANDLER, \
                SUM({amt}) AS OFFEN, CAST(COUNT(*) AS int) AS N, \
                MIN(NULLIF(LTRIM(RTRIM(ISNULL(b.[{d}],''))),'')) AS OLDEST \
         FROM BEH b \
         JOIN LBLOCKENTRY e ON LTRIM(RTRIM(e.[{eb}])) = LTRIM(RTRIM(b.[{bb}])) \
         WHERE ISNULL(b.[{d}],'') >= @P1 \
           AND (LTRIM(RTRIM(ISNULL(b.[{bl}],''))) = '' \
                OR NOT EXISTS (SELECT 1 FROM BILL r \
                               WHERE LTRIM(RTRIM(r.[{bk}])) = LTRIM(RTRIM(b.[{bl}])))) \
         GROUP BY LTRIM(RTRIM(ISNULL(b.[{beh}],'')))",
        beh = ident(&m.beh_behandler),
        d = ident(&m.beh_datum),
        eb = ident(&m.lblockentry_block),
        bb = ident(&m.beh_block),
        bl = ident(&m.beh_bill),
        bk = ident(&m.bill_key),
    );
    let rows = conn.rows(&sql, &[&cutoff]).await?;
    let mut out: Vec<OpenServicesRow> = rows
        .iter()
        .map(|r| OpenServicesRow {
            snapshot_date: snapshot_date.to_string(),
            behandler: get_str(r, "BEHANDLER"),
            offen_betrag: round2(r.get::<f64, _>("OFFEN").unwrap_or(0.0)),
            n: i64::from(r.get::<i32, _>("N").unwrap_or(0)),
            oldest: ymd_to_iso(&get_str(r, "OLDEST")),
        })
        .collect();
    out.sort_by(|a, b| a.behandler.cmp(&b.behandler));
    Ok(out)
}

// ── Sync-Zyklus ──────────────────────────────────────────────────────────────

/// Ein kompletter Sync: Discovery → Gates → Aggregate → Push an die Cloud.
async fn sync_once(cfg: &ConnectorConfig, cloud: &CloudClient) -> Result<()> {
    let mut conn = z1db::connect(
        &cfg.z1_db_server,
        &cfg.z1_db_database,
        &cfg.z1_db_user,
        &cfg.z1_db_password,
        cfg.z1_db_trust_cert,
    )
    .await?;

    let disc = discover_schema(&mut conn).await?;
    let map = ColumnMap::resolved(cfg.z1_control_column_map.as_ref());

    let today = chrono::Local::now().date_naive();
    let snapshot_date = today.format("%Y-%m-%d").to_string();
    // Fenster: letzte N Monate, ab Monatserstem (Datums-Strings JJJJMMTT →
    // lexikografischer Vergleich funktioniert).
    let cutoff = today
        .checked_sub_months(chrono::Months::new(cfg.z1_control_months.max(1)))
        .map(|d| d.format("%Y%m01").to_string())
        .unwrap_or_else(|| "19000101".to_string());

    let mut pending = serde_json::Map::new();
    let mut rows_scanned: i64 = 0;
    let mut revenue: Vec<RevenueRow> = Vec::new();
    let mut ar_aging: Vec<ArAgingRow> = Vec::new();
    let mut open_services: Vec<OpenServicesRow> = Vec::new();

    // revenue: BEH ⋈ LBLOCKENTRY.
    let req: Vec<(&str, &str)> = vec![
        ("BEH", &map.beh_patnr),
        ("BEH", &map.beh_datum),
        ("BEH", &map.beh_art),
        ("BEH", &map.beh_behandler),
        ("BEH", &map.beh_block),
        ("LBLOCKENTRY", &map.lblockentry_block),
        ("LBLOCKENTRY", &map.lblockentry_betrag),
    ];
    let missing = disc.missing(&req);
    if missing.is_empty() {
        rows_scanned += count_beh_since(&mut conn, &map, &cutoff).await.unwrap_or(0);
        revenue = query_revenue(&mut conn, &map, &cutoff).await?;
    } else {
        pending.insert(
            "revenue".into(),
            pending_entry(missing, &disc, &["BEH", "LBLOCKENTRY"]),
        );
    }

    // payments: KONTO + CASH, je Quelle eigenes Gate (eine Quelle darf liefern,
    // während die andere noch auf ihr Mapping wartet).
    let req_konto: Vec<(&str, &str)> = vec![
        ("KONTO", &map.konto_datum),
        ("KONTO", &map.konto_betrag),
        ("KONTO", &map.konto_zahlart),
    ];
    let req_cash: Vec<(&str, &str)> = vec![
        ("CASH", &map.cash_datum),
        ("CASH", &map.cash_betrag),
        ("CASH", &map.cash_zahlart),
    ];
    let (miss_konto, miss_cash) = (disc.missing(&req_konto), disc.missing(&req_cash));
    let mut pay_acc: BTreeMap<(String, String), (f64, i64)> = BTreeMap::new();
    if miss_konto.is_empty() {
        query_payments_source(
            &mut conn,
            "KONTO",
            &map.konto_datum,
            &map.konto_betrag,
            &map.konto_zahlart,
            &cutoff,
            &mut pay_acc,
        )
        .await?;
    }
    if miss_cash.is_empty() {
        query_payments_source(
            &mut conn,
            "CASH",
            &map.cash_datum,
            &map.cash_betrag,
            &map.cash_zahlart,
            &cutoff,
            &mut pay_acc,
        )
        .await?;
    }
    if !miss_konto.is_empty() || !miss_cash.is_empty() {
        let mut missing = miss_konto;
        missing.extend(miss_cash);
        pending.insert(
            "payments".into(),
            pending_entry(missing, &disc, &["KONTO", "CASH"]),
        );
    }
    let payments: Vec<PaymentRow> = pay_acc
        .into_iter()
        .map(|((period, art), (eingang, n))| PaymentRow {
            period,
            art,
            eingang: round2(eingang),
            n,
        })
        .collect();

    // ar_aging: FAKT − KONTO.
    let req: Vec<(&str, &str)> = vec![
        ("FAKT", &map.fakt_rechnr),
        ("FAKT", &map.fakt_datum),
        ("FAKT", &map.fakt_betrag),
        ("KONTO", &map.konto_rechnr),
        ("KONTO", &map.konto_betrag),
    ];
    let missing = disc.missing(&req);
    if missing.is_empty() {
        ar_aging = query_ar_aging(&mut conn, &map, &snapshot_date, today).await?;
    } else {
        pending.insert(
            "ar_aging".into(),
            pending_entry(missing, &disc, &["FAKT", "KONTO"]),
        );
    }

    // open_services: BEH ohne BILL.
    let req: Vec<(&str, &str)> = vec![
        ("BEH", &map.beh_datum),
        ("BEH", &map.beh_behandler),
        ("BEH", &map.beh_block),
        ("BEH", &map.beh_bill),
        ("LBLOCKENTRY", &map.lblockentry_block),
        ("LBLOCKENTRY", &map.lblockentry_betrag),
        ("BILL", &map.bill_key),
    ];
    let missing = disc.missing(&req);
    if missing.is_empty() {
        open_services = query_open_services(&mut conn, &map, &cutoff, &snapshot_date).await?;
    } else {
        pending.insert(
            "open_services".into(),
            pending_entry(missing, &disc, &["BEH", "LBLOCKENTRY", "BILL"]),
        );
    }

    let status = if pending.is_empty() {
        "ok"
    } else if pending.len() >= 4 {
        "pending_mapping"
    } else {
        "partial"
    };
    let report = ControlReport {
        sync: ControlSync {
            watermark: snapshot_date.clone(),
            status: status.to_string(),
            rows_scanned,
            schema: disc.schema_json(),
            pending_mappings: serde_json::Value::Object(pending),
        },
        revenue,
        payments,
        ar_aging,
        open_services,
    };
    cloud.report_control(&report).await?;
    info!(
        status,
        revenue = report.revenue.len(),
        payments = report.payments.len(),
        ar_aging = report.ar_aging.len(),
        open_services = report.open_services.len(),
        "Praxis-Steuerungs-Report an die Cloud gemeldet"
    );
    Ok(())
}

// ── Tages-Marker + Zeitfenster ───────────────────────────────────────────────

/// Liegt `now_hour` im Fenster `target ± 1` (mit Tages-Überlauf)?
/// Anacron-Prinzip statt fixem Cron: Der Sync soll **einmal pro Kalendertag** laufen,
/// und zwar am oder nach der frühesten Stunde `earliest_hour` beim ersten Tick, an dem
/// der PC an ist. Ist der Rechner zur Zielstunde aus (nachts der Normalfall in Praxen),
/// wird der Lauf am Morgen NACHGEHOLT statt ausgelassen; ein 24/7-Mini-PC trifft die
/// Stunde exakt (Nebenlast). `earliest_hour` muss ≤ Öffnungszeit sein (Default 3),
/// sonst käme der PC nie in ein Zeitfenster ≥ earliest_hour und der Lauf bliebe aus.
fn should_run(now: &chrono::DateTime<chrono::Local>, earliest_hour: u8, last_run: Option<&str>) -> bool {
    let today = now.format("%Y-%m-%d").to_string();
    if last_run == Some(today.as_str()) {
        return false; // heute schon erfolgreich gelaufen → kein Doppellauf
    }
    now.hour() >= u32::from(earliest_hour % 24)
}

fn last_run_date() -> Option<String> {
    paths::control_last_run_file()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

fn mark_ran(date: &str) {
    if let Ok(p) = paths::control_last_run_file() {
        let _ = std::fs::write(p, date);
    }
}

/// Startet den täglichen Praxis-Steuerungs-Sync als eigenständige Schleife
/// (Muster `hkp::spawn`). Tickt stündlich; führt den Sync einmal pro Tag
/// (persistierter Marker) am oder nach `z1_control_hour` aus und holt einen in der
/// Nacht (PC aus) verpassten Lauf am Morgen nach (siehe [`should_run`]).
pub fn spawn(cfg: ConnectorConfig) -> LoopHandle {
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        if !cfg.z1_control_enabled {
            info!("Praxis-Steuerungs-Sync deaktiviert (z1_control_enabled=false)");
            return;
        }
        let cloud = match CloudClient::new(&cfg) {
            Ok(c) => c,
            Err(e) => {
                warn!(error=%e, "Praxis-Steuerungs-Sync: Cloud-Client fehlgeschlagen — Schleife beendet");
                return;
            }
        };
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        info!(
            stunde = cfg.z1_control_hour,
            monate = cfg.z1_control_months,
            "Praxis-Steuerungs-Sync gestartet (täglich)"
        );
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let now = chrono::Local::now();
                    if !should_run(&now, cfg.z1_control_hour, last_run_date().as_deref()) {
                        continue;
                    }
                    let today = now.format("%Y-%m-%d").to_string();
                    match tokio::time::timeout(CYCLE_TIMEOUT, sync_once(&cfg, &cloud)).await {
                        // Marker (Datum) nur bei Erfolg → ein Fehlschlag (Z1/Netz kurz weg)
                        // wird zur nächsten vollen Stunde erneut versucht, den ganzen Tag über.
                        Ok(Ok(())) => mark_ran(&today),
                        Ok(Err(e)) => warn!(error=%e, "Praxis-Steuerungs-Sync fehlgeschlagen — Retry zur nächsten Stunde"),
                        Err(_) => warn!("Praxis-Steuerungs-Sync abgebrochen (Timeout 300 s) — Retry zur nächsten Stunde"),
                    }
                }
                _ = rx.changed() => {
                    if *rx.borrow() { info!("Praxis-Steuerungs-Sync gestoppt"); break; }
                }
            }
        }
    });
    LoopHandle::new(tx, join)
}

// ── Tests (reine Logik, kein DB-Zugriff) ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn disc_with(pairs: &[(&str, &str)]) -> Discovery {
        Discovery::from_pairs(
            pairs
                .iter()
                .map(|(t, c)| (t.to_string(), c.to_string())),
        )
    }

    #[test]
    fn gate_fehlende_spalte_erzeugt_pending_mapping() {
        // BEH hat DATUM, aber kein LEISTUNGSART → Teil muss ausgelassen werden.
        let disc = disc_with(&[
            ("BEH", "PATNR"),
            ("BEH", "DATUM"),
            ("BEH", "SONSTWAS"),
            ("LBLOCKENTRY", "EINZELBETRAG"),
        ]);
        let m = ColumnMap::default();
        let req: Vec<(&str, &str)> = vec![
            ("BEH", &m.beh_patnr),
            ("BEH", &m.beh_datum),
            ("BEH", &m.beh_art),
            ("LBLOCKENTRY", &m.lblockentry_betrag),
        ];
        let missing = disc.missing(&req);
        assert_eq!(missing, vec!["BEH.LEISTUNGSART".to_string()]);

        let entry = pending_entry(missing, &disc, &["BEH", "LBLOCKENTRY"]);
        assert_eq!(entry["missing"][0], "BEH.LEISTUNGSART");
        let avail: Vec<String> = entry["available"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(avail.contains(&"BEH.PATNR".to_string()));
        assert!(avail.contains(&"BEH.SONSTWAS".to_string()));
        assert!(avail.contains(&"LBLOCKENTRY.EINZELBETRAG".to_string()));

        // Gate grün, wenn alles da ist (case-insensitiv).
        assert!(disc.has("BEH", "patnr"));
        assert!(disc
            .missing(&[("BEH", "PATNR"), ("BEH", "DATUM")])
            .is_empty());
    }

    #[test]
    fn monats_ableitung_aus_jjjjmmtt() {
        assert_eq!(period_from_ym("202607"), Some("2026-07-01".into()));
        assert_eq!(period_from_ym("20260708"), Some("2026-07-01".into())); // volle JJJJMMTT
        assert_eq!(period_from_ym(" 202612 "), Some("2026-12-01".into()));
        assert_eq!(period_from_ym("202613"), None); // Monat 13
        assert_eq!(period_from_ym("2026"), None); // zu kurz
        assert_eq!(period_from_ym(""), None);
        assert_eq!(period_from_ym("ABCDEF"), None);
        assert_eq!(ymd_to_iso("20230704"), Some("2023-07-04".into()));
        assert_eq!(ymd_to_iso("00000000"), None);
        assert_eq!(ymd_to_iso(""), None);
    }

    #[test]
    fn betrag_parsing_deutsch_und_punkt() {
        assert_eq!(parse_amount("1.234,56"), 1234.56);
        assert_eq!(parse_amount("1234.56"), 1234.56);
        assert_eq!(parse_amount("1234,56"), 1234.56);
        assert_eq!(parse_amount("0"), 0.0);
        assert_eq!(parse_amount(""), 0.0);
        assert_eq!(parse_amount("  -12,50 "), -12.5);
        assert_eq!(parse_amount("Müll"), 0.0);
    }

    #[test]
    fn aging_bucket_zuordnung() {
        assert_eq!(bucket_for(0), "0-30");
        assert_eq!(bucket_for(30), "0-30");
        assert_eq!(bucket_for(31), "31-60");
        assert_eq!(bucket_for(60), "31-60");
        assert_eq!(bucket_for(61), "61-90");
        assert_eq!(bucket_for(90), "61-90");
        assert_eq!(bucket_for(91), "90+");
        assert_eq!(bucket_for(1000), "90+");
        assert_eq!(bucket_for(-5), "0-30"); // Zukunftsdatum → jüngster Bucket
    }

    #[test]
    fn report_serialisierung_feldnamen_exakt() {
        let report = ControlReport {
            sync: ControlSync {
                watermark: "2026-07-08".into(),
                status: "partial".into(),
                rows_scanned: 42,
                schema: serde_json::json!({"BEH": ["PATNR"]}),
                pending_mappings: serde_json::json!({"revenue": {"missing": ["BEH.X"], "available": []}}),
            },
            revenue: vec![RevenueRow {
                period: "2026-07-01".into(),
                art: "bema".into(),
                gruppe: None,
                behandler: "ak".into(),
                standort: None,
                honorar: 1234.56,
                eigenlabor: None,
                fremdlabor: None,
                n_leistungen: 10,
                n_faelle: 3,
            }],
            payments: vec![PaymentRow {
                period: "2026-07-01".into(),
                art: "ueberweisung".into(),
                eingang: 99.5,
                n: 2,
            }],
            ar_aging: vec![ArAgingRow {
                snapshot_date: "2026-07-08".into(),
                bucket: "0-30".into(),
                offen: 500.0,
                n: 4,
            }],
            open_services: vec![OpenServicesRow {
                snapshot_date: "2026-07-08".into(),
                behandler: "ak".into(),
                offen_betrag: 321.09,
                n: 7,
                oldest: Some("2026-01-02".into()),
            }],
        };
        let v = serde_json::to_value(&report).unwrap();
        // sync
        assert_eq!(v["sync"]["watermark"], "2026-07-08");
        assert_eq!(v["sync"]["status"], "partial");
        assert_eq!(v["sync"]["rows_scanned"], 42);
        assert_eq!(v["sync"]["schema"]["BEH"][0], "PATNR");
        assert_eq!(v["sync"]["pending_mappings"]["revenue"]["missing"][0], "BEH.X");
        // revenue
        let r = &v["revenue"][0];
        assert_eq!(r["period"], "2026-07-01");
        assert_eq!(r["art"], "bema");
        assert!(r["gruppe"].is_null());
        assert_eq!(r["behandler"], "ak");
        assert!(r["standort"].is_null());
        assert_eq!(r["honorar"], 1234.56);
        assert!(r["eigenlabor"].is_null());
        assert!(r["fremdlabor"].is_null());
        assert_eq!(r["n_leistungen"], 10);
        assert_eq!(r["n_faelle"], 3);
        // payments
        let p = &v["payments"][0];
        assert_eq!(p["period"], "2026-07-01");
        assert_eq!(p["art"], "ueberweisung");
        assert_eq!(p["eingang"], 99.5);
        assert_eq!(p["n"], 2);
        // ar_aging
        let a = &v["ar_aging"][0];
        assert_eq!(a["snapshot_date"], "2026-07-08");
        assert_eq!(a["bucket"], "0-30");
        assert_eq!(a["offen"], 500.0);
        assert_eq!(a["n"], 4);
        // open_services
        let o = &v["open_services"][0];
        assert_eq!(o["snapshot_date"], "2026-07-08");
        assert_eq!(o["behandler"], "ak");
        assert_eq!(o["offen_betrag"], 321.09);
        assert_eq!(o["n"], 7);
        assert_eq!(o["oldest"], "2026-01-02");
    }

    #[test]
    fn column_map_override_wird_angewendet() {
        let over = serde_json::json!({
            "beh_datum": "LEISTUNGSDATUM",
            "kaputt": 42, // Nicht-String + unbekannte Keys werden ignoriert
        });
        let m = ColumnMap::resolved(Some(&over));
        assert_eq!(m.beh_datum, "LEISTUNGSDATUM");
        assert_eq!(m.beh_patnr, "PATNR"); // Default bleibt
        assert_eq!(ColumnMap::resolved(None).beh_datum, "DATUM");
    }

    #[test]
    fn should_run_holt_verpassten_nachtlauf_nach() {
        use chrono::{Local, TimeZone};
        let at = |h: u32| Local.with_ymd_and_hms(2026, 7, 9, h, 0, 0).unwrap();
        let heute = "2026-07-09";
        let gestern = "2026-07-08";

        // 24/7-Mini-PC: um 3 Uhr an, heute noch nicht gelaufen → läuft (Nebenlast nachts).
        assert!(should_run(&at(3), 3, Some(gestern)));
        // Normaler Praxis-PC: um 3 Uhr aus, bootet um 8 Uhr → Nachtlauf wird nachgeholt.
        assert!(should_run(&at(8), 3, Some(gestern)));
        // Vor der frühesten Stunde (1 Uhr, PC lief lange) → noch nicht.
        assert!(!should_run(&at(1), 3, Some(gestern)));
        // Heute schon erfolgreich gelaufen → kein Doppellauf, auch wenn PC den Tag über an ist.
        assert!(!should_run(&at(8), 3, Some(heute)));
        assert!(!should_run(&at(18), 3, Some(heute)));
        // Noch nie gelaufen, Morgen → läuft.
        assert!(should_run(&at(9), 3, None));
    }

    #[test]
    fn map_art_kanonisch_und_durchreichen() {
        assert_eq!(map_art(" BEMA "), "bema");
        assert_eq!(map_art("GOZ"), "goz");
        assert_eq!(map_art("Privat"), "privat");
        assert_eq!(map_art("EB"), "eb"); // unbekannt → roh (klein) durchreichen
        assert_eq!(map_art(""), "unbekannt");
    }

    #[test]
    fn sql_amount_und_ident_haertung() {
        let e = sql_amount("e.[BETRAG]");
        assert!(e.contains("TRY_CAST"));
        assert!(e.contains("LIKE '%,%'"));
        assert_eq!(ident("EINZELBETRAG"), "EINZELBETRAG");
        assert_eq!(ident("X]; DROP TABLE PAT;--"), "XDROPTABLEPAT");
    }
}
