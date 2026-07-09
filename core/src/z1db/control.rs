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
/// (LEB/PERSONAL für die Behandler-Auflösung; GO/PUNKTWERTE für den KCH-Punktwert;
/// ZPLAN/PARHIT/ZEHIT/KBRHIT für die GKV-Sparten PAR/ZE/KBR je Behandler.)
const DISCOVERY_TABLES: [&str; 15] = [
    "BEH", "LBLOCK", "LBLOCKENTRY", "BILL", "FAKT", "KONTO", "CASH", "LEB", "PERSONAL",
    "GO", "PUNKTWERTE", "ZPLAN", "PARHIT", "KBRHIT", "ZEHIT",
];

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

/// SQL-Ausdruck, der einen Z1-Betrag nach EUR wandelt. **Z1-Geldformat (DB-weit,
/// am ZMM verifiziert):** Währungspräfix `e` (oder Leerzeichen) + Cent-Ganzzahl,
/// KEIN Dezimaltrenner — `"e 225992"` = 2.259,92 €. `e` + Leerzeichen strippen,
/// verbleibende Ziffern als `bigint` (= Cent) lesen, `/ 100`. Leer → NULL → 0.
/// Siehe `docs/Z1-BILLING.md` §1.
fn sql_amount(expr: &str) -> String {
    format!(
        "(ISNULL(CAST(NULLIF(REPLACE(REPLACE({e},'e',''),' ',''),'') AS bigint), 0) / 100.0)",
        e = expr
    )
}

/// Verpackt einen Float-Betragsausdruck als **Fixkomma-String** (kein wissen-
/// schaftliches Format). Grund: der tiberius-Treiber liefert `SUM(float)`/float
/// per Spaltenname als 0 zurück (Integer- und String-Reads funktionieren dagegen
/// korrekt — am ZMM verifiziert: n_leistungen stimmte, honorar war 0). Deshalb den
/// Betrag als `decimal(19,2)`→`varchar` zurückgeben und in Rust mit [`parse_amount`]
/// parsen.
fn as_amount_str(float_expr: &str) -> String {
    format!("CONVERT(varchar(40), CAST({float_expr} AS decimal(19,2)))")
}

/// Parst einen (SQL-seitig als String gelieferten) Betrag robust nach `f64` —
/// deutsche Komma- wie Punkt-Notation. Produktiv genutzt (s. [`as_amount_str`]).
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

/// `"MM.JJJJ"` → Monats-Periode `"JJJJ-MM-01"` (None bei Müll). Format der
/// Sammelrechnungs-/Plan-Perioden (BESCHREIBUNG-Suffix, `DTADATUM`-Ableitung).
fn period_from_mmjjjj(s: &str) -> Option<String> {
    let t = s.trim();
    if t.len() != 7 || t.as_bytes()[2] != b'.' {
        return None;
    }
    let (mm, jjjj) = (&t[..2], &t[3..]);
    if !mm.chars().all(|c| c.is_ascii_digit()) || !jjjj.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let month: u32 = mm.parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some(format!("{jjjj}-{mm}-01"))
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

/// `BEH.GOART` → Abrechnungsart (docs/Z1-BILLING.md §3): `g`=BEMA/GKV, `q`=GOZ,
/// `2/3/4/7`=Privat-Material/Sonderpositionen. Alles andere unverändert.
fn map_goart(goart: &str) -> String {
    match goart.trim().to_lowercase().as_str() {
        "g" => "bema".into(),
        "q" => "goz".into(),
        "2" | "3" | "4" | "7" => "privat".into(),
        "" => "unbekannt".into(),
        other => other.into(),
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
    /// Leistungsart (BEMA/GOZ/privat …). ZMM-real: `GOART`.
    pub beh_art: String,
    /// Honorar-Betrag je erbrachter Leistung (ZMM-real: `DMBETRAG`, trotz Alt-Name
    /// der Euro-Betrag). Umsatz = Summe hierüber; kein Join zu LBLOCKENTRY nötig.
    pub beh_betrag: String,
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
    // KONTO = Kontenrahmen (KEINE Zahlungen) → Felder ungenutzt, nur Rückwärtskompat.
    pub konto_datum: String,
    pub konto_betrag: String,
    pub konto_zahlart: String,
    pub konto_rechnr: String,
    // CASH (Zahlungstransaktionen — die echte Zahlungsquelle)
    pub cash_datum: String,
    pub cash_betrag: String,
    pub cash_zahlart: String,
    /// Storno-Kennzeichen; gesetzt = stornierte Zahlung (ausgeschlossen).
    pub cash_storno: String,
    // FAKT (Rechnungen — offene Forderungen)
    pub fakt_rechnr: String,
    pub fakt_datum: String,
    pub fakt_betrag: String,
    /// Beglichen-Kennzeichen; leer = offen, gesetzt (Datum/Flag) = bezahlt.
    pub fakt_offen: String,
    /// Storno-Kennzeichen der Rechnung (ausgeschlossen).
    pub fakt_storno: String,
    // BILL (Abrechnungen)
    pub bill_key: String,
}

impl Default for ColumnMap {
    fn default() -> Self {
        Self {
            // Defaults = am ZMM-Piloten (09.07.2026) per Schema-Discovery verifizierte
            // echte Z1-Spalten. Andere Z1-Installationen ggf. via z1_control_column_map.
            beh_patnr: "PATNR".into(),
            beh_datum: "DATUM".into(),
            beh_art: "GOART".into(),
            beh_betrag: "DMBETRAG".into(),
            beh_ziffer: "LSTNR".into(),
            beh_behandler: "LEBID".into(),
            beh_block: "LFDLBLOCK".into(),   // (Reserve; Umsatz braucht keinen Join mehr)
            beh_bill: "LFDPATBILL".into(),   // leer = noch nicht abgerechnet (liegengeblieben)
            lblockentry_block: "LFDLBLOCK".into(),
            lblockentry_betrag: "EINZELBETRAG".into(),
            konto_datum: "".into(),          // KONTO = Kontenrahmen, NICHT Zahlungen → ungenutzt
            konto_betrag: "".into(),
            konto_zahlart: "".into(),
            konto_rechnr: "".into(),
            cash_datum: "CASHDATUM".into(),
            cash_betrag: "BETRAG".into(),
            cash_zahlart: "ZAHLUNGSWEG".into(),
            cash_storno: "STORNIERT".into(),
            fakt_rechnr: "LFDFAKT".into(),
            fakt_datum: "FAKTDATUM".into(),
            fakt_betrag: "BETRAG".into(),
            fakt_offen: "BEGLICHEN".into(),  // leer = offen; gesetzt (Datum/Flag) = beglichen
            fakt_storno: "STORNIERT".into(),
            bill_key: "LFDPATBILL".into(),
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

/// Behandler-Auflösung `BEH.LEBID → LEB.PID → PERSONAL.KUERZEL` (docs/Z1-BILLING.md
/// §3). Fallback = LEBID, falls kein Name. `LEB.BEZEICHNUNG` ist leer → nicht nutzen.
const BEHANDLER_JOIN: &str =
    "LEFT JOIN LEB lb ON LTRIM(RTRIM(lb.LEBID)) = LTRIM(RTRIM(b.[{leb}])) \
     LEFT JOIN PERSONAL pe ON LTRIM(RTRIM(pe.PID)) = LTRIM(RTRIM(lb.PID))";

/// Honorarumsatz je Monat × Behandler × Abrechnungsart aus **fünf überschneidungs-
/// freien Quellen** (docs/Z1-BILLING.md §4, am ZMM faktura-reconciled):
///   1. **Privat/GOZ** (GOART q/2/3/4/7): exakt aus `BEH.DMBETRAG`, je Monat × Behandler.
///   2. **KCH** (art=bema, gruppe=KCH): BEH-BEMA (GOART 'g') × GO-Gebührenpunkt ×
///      aktueller Punktwert, je Monat × Behandler (BEH.LEBID).
///   3. **PAR** (art=bema, gruppe=PAR): Einzelrechnungen (FAKT RART 5050) + Sammel-
///      rechnung `0kp` pro-rata auf PARHIT-Pläne, Behandler = ZPLAN.LEBID.
///   4. **ZE**  (art=bema, gruppe=ZE):  Einzelrechnungen (FAKT RART 5060/6020) +
///      Sammelrechnung `0kz` pro-rata auf ZEHIT-Pläne, Behandler = ZPLAN.LEBID.
///   5. **KBR** (art=bema, gruppe=KBR): Sammelrechnung `0kb` pro-rata auf KBRHIT-Pläne.
///
/// Behandler-Achse: KCH/GOZ über `BEH.LEBID`; ZE/PAR/KBR IMMER über den Plan
/// (`ZPLAN.LEBID`), NIE über die Rechnung. Alle Schlüssel mit `LTRIM(RTRIM())`.
async fn query_revenue(
    conn: &mut Z1Connection,
    m: &ColumnMap,
    cutoff: &str,
) -> Result<Vec<RevenueRow>> {
    let mut out: Vec<RevenueRow> = Vec::new();

    // 1) Privat/GOZ aus BEH je Monat × Behandler × GOART.
    let honorar = as_amount_str(&format!("SUM({})", sql_amount(&format!("b.[{}]", ident(&m.beh_betrag)))));
    let beh_join = BEHANDLER_JOIN.replace("{leb}", &ident(&m.beh_behandler));
    let sql_privat = format!(
        "SELECT SUBSTRING(ISNULL(b.[{d}],''),1,6) AS YM, \
                LTRIM(RTRIM(ISNULL(b.[{a}],''))) AS ART, \
                ISNULL(NULLIF(LTRIM(RTRIM(pe.KUERZEL)),''), LTRIM(RTRIM(b.[{leb}]))) AS BEHANDLER, \
                {honorar} AS HONORAR, \
                CAST(COUNT(*) AS int) AS N_LEIST, \
                CAST(COUNT(DISTINCT LTRIM(RTRIM(b.[{p}]))) AS int) AS N_FAELLE \
         FROM BEH b {beh_join} \
         WHERE ISNULL(b.[{d}],'') >= @P1 \
           AND LTRIM(RTRIM(ISNULL(b.[{a}],''))) NOT IN ('g','') \
         GROUP BY SUBSTRING(ISNULL(b.[{d}],''),1,6), LTRIM(RTRIM(ISNULL(b.[{a}],''))), \
                  ISNULL(NULLIF(LTRIM(RTRIM(pe.KUERZEL)),''), LTRIM(RTRIM(b.[{leb}])))",
        d = ident(&m.beh_datum),
        a = ident(&m.beh_art),
        leb = ident(&m.beh_behandler),
        p = ident(&m.beh_patnr),
    );
    for r in &conn.rows(&sql_privat, &[&cutoff]).await? {
        let Some(period) = period_from_ym(&get_str(r, "YM")) else { continue };
        out.push(RevenueRow {
            period,
            art: map_goart(&get_str(r, "ART")),
            gruppe: None,
            behandler: get_str(r, "BEHANDLER"),
            standort: None,
            honorar: round2(parse_amount(&get_str(r, "HONORAR"))),
            eigenlabor: None,
            fremdlabor: None,
            n_leistungen: i64::from(r.get::<i32, _>("N_LEIST").unwrap_or(0)),
            n_faelle: i64::from(r.get::<i32, _>("N_FAELLE").unwrap_or(0)),
        });
    }

    // 2)–5) GKV je Behandler nach Sparte (KCH/PAR/ZE/KBR), ersetzt den früheren
    //        FAKT.ZHON-GKV-Gesamtwert (der keinen Behandler kannte).
    out.extend(query_revenue_kch(conn, m, cutoff).await?);
    out.extend(query_revenue_par(conn, m, cutoff).await?);
    out.extend(query_revenue_ze(conn, m, cutoff).await?);
    out.extend(query_revenue_kbr(conn, m, cutoff).await?);

    out.sort_by(|a, b| {
        (&a.period, &a.art, &a.gruppe, &a.behandler).cmp(&(&b.period, &b.art, &b.gruppe, &b.behandler))
    });
    Ok(out)
}

/// **Behandler-Auflösung über den Plan** (`ZPLAN.LEBID → LEB.PID → PERSONAL.KUERZEL`).
/// Für ZE/PAR/KBR gilt der behandelnde Arzt des HKP/Plans, NICHT der Rechnungssteller.
/// Der Aufrufer muss `zp.LEBID` bereitstellen (via OUTER APPLY je Fall).
const PLAN_BEHANDLER_JOIN: &str =
    "LEFT JOIN LEB lb ON LTRIM(RTRIM(lb.LEBID)) = LTRIM(RTRIM(zp.LEBID)) \
     LEFT JOIN PERSONAL pe ON LTRIM(RTRIM(pe.PID)) = LTRIM(RTRIM(lb.PID))";

/// SQL-Ausdruck Behandler-Kürzel (Fallback = das rohe LEBID-Feld `raw_lebid`).
fn behandler_expr(raw_lebid: &str) -> String {
    format!("ISNULL(NULLIF(LTRIM(RTRIM(pe.KUERZEL)),''), LTRIM(RTRIM({raw_lebid})))")
}

/// **KCH** (art=bema, gruppe=KCH): konservierende/chirurgische BEMA-Leistungen.
/// Honorar = Σ (GO-Gebührenpunkt × Anzahl × aktueller Punktwert) je Monat × Behandler.
/// GO-Punktwert (`pw`) = aktuellster Wert je PWLART (ABDATUM ≤ heute); je BEH-Zeile
/// die zum Leistungsdatum gültige GO-Position (ABDATUM ≤ DATUM, BISDATUM offen/≥ DATUM).
async fn query_revenue_kch(
    conn: &mut Z1Connection,
    m: &ColumnMap,
    cutoff: &str,
) -> Result<Vec<RevenueRow>> {
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    // Punktwert (€/Punkt) = euro(PWWEST)/10000 (PWWEST = Cent × 100). Aktuellster je
    // PWLART: Zeilen mit ABDATUM ≤ heute, davon AVG (i. d. R. genau eine gültige Zeile).
    let pw_euro = sql_amount("PWWEST");
    let behandler = behandler_expr(&format!("b.[{}]", ident(&m.beh_behandler)));
    let beh_join = BEHANDLER_JOIN.replace("{leb}", &ident(&m.beh_behandler));
    // GEBPKT/100 (Gebührenpunkte) × ANZAHL/100 × Punktwert(€). GEBPKT/ANZAHL sind
    // Hundertstel-Ganzzahlen (kein 'e'-Präfix); Punktwert = PWWEST(Cent)/10000.
    let honorar_f = "SUM( \
        (CAST(NULLIF(REPLACE(REPLACE(g.GEBPKT,'e',''),' ',''),'') AS float)/100.0) \
        * (CAST(NULLIF(REPLACE(REPLACE(b.[{anz}],'e',''),' ',''),'') AS float)/100.0) \
        * ISNULL(pw.pwert, 0) )";
    let honorar_f = honorar_f.replace("{anz}", &ident("ANZAHL"));
    let honorar = as_amount_str(&honorar_f);
    let sql = format!(
        "WITH pw AS ( \
            SELECT LTRIM(RTRIM(PWLART)) AS PWLART, \
                   AVG(({pw_euro})/10000.0) AS pwert \
            FROM PUNKTWERTE \
            WHERE LTRIM(RTRIM(ISNULL(ABDATUM,''))) <> '' AND ABDATUM <= '{today}' \
            GROUP BY LTRIM(RTRIM(PWLART)) \
        ) \
        SELECT SUBSTRING(ISNULL(b.[{d}],''),1,6) AS YM, \
               {behandler} AS BEHANDLER, \
               {honorar} AS HONORAR, \
               CAST(COUNT(*) AS int) AS N_LEIST, \
               CAST(COUNT(DISTINCT LTRIM(RTRIM(b.[{p}]))) AS int) AS N_FAELLE \
        FROM BEH b {beh_join} \
        CROSS APPLY ( \
            SELECT TOP 1 g.GEBPKT, g.PWLART FROM GO g \
            WHERE LTRIM(RTRIM(g.KYLSTNR)) = LTRIM(RTRIM(b.[{kyl}])) \
              AND g.ABDATUM <= b.[{d}] \
              AND (g.BISDATUM IS NULL OR LTRIM(RTRIM(g.BISDATUM)) = '' OR g.BISDATUM >= b.[{d}]) \
              AND LTRIM(RTRIM(ISNULL(g.GEBPKT,''))) <> '' \
            ORDER BY g.ABDATUM DESC \
        ) g \
        LEFT JOIN pw ON pw.PWLART = LTRIM(RTRIM(g.PWLART)) \
        WHERE LTRIM(RTRIM(ISNULL(b.[{a}],''))) = 'g' \
          AND ISNULL(b.[{d}],'') >= @P1 \
        GROUP BY SUBSTRING(ISNULL(b.[{d}],''),1,6), {behandler}",
        d = ident(&m.beh_datum),
        a = ident(&m.beh_art),
        p = ident(&m.beh_patnr),
        kyl = ident("KYLSTNR"),
    );
    let mut out: Vec<RevenueRow> = Vec::new();
    for r in &conn.rows(&sql, &[&cutoff]).await? {
        let Some(period) = period_from_ym(&get_str(r, "YM")) else { continue };
        out.push(RevenueRow {
            period,
            art: "bema".into(),
            gruppe: Some("KCH".into()),
            behandler: get_str(r, "BEHANDLER"),
            standort: None,
            honorar: round2(parse_amount(&get_str(r, "HONORAR"))),
            eigenlabor: None,
            fremdlabor: None,
            n_leistungen: i64::from(r.get::<i32, _>("N_LEIST").unwrap_or(0)),
            n_faelle: i64::from(r.get::<i32, _>("N_FAELLE").unwrap_or(0)),
        });
    }
    Ok(out)
}

/// SQL-CTE `sammel`: Sammelrechnungs-Honorar (`FAKT.ZHON`) je Monatsperiode für ein
/// Sammel-Kto (`0kz`/`0kp`/`0kb`). Periode = `MM.JJJJ`-Suffix der BESCHREIBUNG.
/// `kto` ist ein Literal aus dieser Funktion (kein User-Input).
fn sammel_cte(kto: &str) -> String {
    let zhon = sql_amount("[ZHON]");
    format!(
        "sammel AS ( \
            SELECT RIGHT(LTRIM(RTRIM([BESCHREIBUNG])),7) AS periode, \
                   SUM({zhon}) AS zhon \
            FROM FAKT \
            WHERE LTRIM(RTRIM([PATNR])) = '{kto}' \
              AND LTRIM(RTRIM(ISNULL([STORNIERT],''))) <> '1' \
              AND LEN(LTRIM(RTRIM(ISNULL([BESCHREIBUNG],'')))) >= 7 \
              AND SUBSTRING(RIGHT(LTRIM(RTRIM([BESCHREIBUNG])),7),3,1) = '.' \
            GROUP BY RIGHT(LTRIM(RTRIM([BESCHREIBUNG])),7) \
        )"
    )
}

/// Pro-rata-Sammelrechnung → RevenueRows je (Periode × Behandler): der Sammelbetrag
/// je Periode wird nach dem Gewicht (Einzel-HKP-Summe) auf die Behandler verteilt.
/// `hit_from` = FROM/APPLY-Klausel, die `periode`, `gewicht` und `LEBID_RAW`
/// bereitstellt (siehe Aufrufer). Muster: sammel → fall → quote (SUM OVER auf
/// eigener Ebene, SQL-Server verbietet SUM(x) OVER() direkt im Aggregat) → final.
async fn query_sammel_prorata(
    conn: &mut Z1Connection,
    cutoff: &str,
    kto: &str,
    gruppe: &str,
    fall_select: &str,
) -> Result<Vec<RevenueRow>> {
    let sammel = sammel_cte(kto);
    let plan_join = PLAN_BEHANDLER_JOIN;
    // Fallback = das rohe Plan-LEBID (im agg-CTE als LEBID exponiert; PLAN_BEHANDLER_JOIN
    // matcht auf zp.LEBID → hier per abgeleitetem Alias `zp` bereitgestellt).
    let behandler = behandler_expr("agg.LEBID");
    // sammel → fall (periode, LEBID_RAW, gewicht) → quote (pro-rata-Quote q, SUM OVER
    // auf eigener Ebene) → agg (Honorar je periode×LEBID) → final (LEBID→KUERZEL).
    let sql = format!(
        "WITH {sammel}, \
         fall AS ( {fall_select} ), \
         quote AS ( \
            SELECT periode, LEBID_RAW, gewicht, \
                   gewicht / NULLIF(SUM(gewicht) OVER (PARTITION BY periode), 0) AS q \
            FROM fall \
         ), \
         agg AS ( \
            SELECT quote.periode AS periode, quote.LEBID_RAW AS LEBID, \
                   SUM(sammel.zhon * quote.q) AS honorar \
            FROM quote \
            JOIN sammel ON sammel.periode = quote.periode \
            GROUP BY quote.periode, quote.LEBID_RAW \
         ) \
         SELECT agg.periode AS PERIODE, \
                {behandler} AS BEHANDLER, \
                {honorar} AS HONORAR \
         FROM agg \
         OUTER APPLY (SELECT agg.LEBID AS LEBID) zp \
         {plan_join} \
         GROUP BY agg.periode, {behandler}",
        behandler = behandler,
        honorar = as_amount_str("SUM(agg.honorar)"),
    );
    let mut out: Vec<RevenueRow> = Vec::new();
    for r in &conn.rows(&sql, &[&cutoff]).await? {
        let Some(period) = period_from_mmjjjj(&get_str(r, "PERIODE")) else { continue };
        out.push(RevenueRow {
            period,
            art: "bema".into(),
            gruppe: Some(gruppe.into()),
            behandler: get_str(r, "BEHANDLER"),
            standort: None,
            honorar: round2(parse_amount(&get_str(r, "HONORAR"))),
            eigenlabor: None,
            fremdlabor: None,
            n_leistungen: 0,
            n_faelle: 0,
        });
    }
    Ok(out)
}

/// **PAR** (art=bema, gruppe=PAR): 1a Einzelrechnungen (FAKT RART 5050, GKV, nicht
/// storniert, numerische PATNR) je Monat × Plan-Behandler; 1b Sammelrechnung `0kp`
/// pro-rata auf die PARHIT-Pläne der jeweiligen Periode.
async fn query_revenue_par(
    conn: &mut Z1Connection,
    m: &ColumnMap,
    cutoff: &str,
) -> Result<Vec<RevenueRow>> {
    let mut out: Vec<RevenueRow> = Vec::new();

    // 1a Einzel: FAKT RART=5050 ⋈ PARHIT ⋈ ZPLAN(→LEBID) → Behandler.
    let honorar = as_amount_str(&format!("SUM({})", sql_amount("f.[ZHON]")));
    let behandler = behandler_expr("zp.LEBID");
    let plan_join = PLAN_BEHANDLER_JOIN;
    let sql_einzel = format!(
        "SELECT SUBSTRING(ISNULL(f.[{fd}],''),1,6) AS YM, \
                {behandler} AS BEHANDLER, \
                {honorar} AS HONORAR, \
                CAST(COUNT(*) AS int) AS N_LEIST, \
                CAST(COUNT(DISTINCT LTRIM(RTRIM(f.[PATNR]))) AS int) AS N_FAELLE \
         FROM FAKT f \
         JOIN PARHIT h ON LTRIM(RTRIM(h.PATNR)) = LTRIM(RTRIM(f.PATNR)) \
                      AND LTRIM(RTRIM(h.LFDPATBILL)) = LTRIM(RTRIM(f.LFDPATBILL)) \
         OUTER APPLY ( \
            SELECT TOP 1 zp.LEBID FROM ZPLAN zp \
            WHERE LTRIM(RTRIM(zp.PATNR)) = LTRIM(RTRIM(h.PATNR)) \
              AND LTRIM(RTRIM(zp.LFDPLAN)) = LTRIM(RTRIM(h.LFDPLAN)) \
         ) zp \
         {plan_join} \
         WHERE LTRIM(RTRIM(ISNULL(f.[RART],''))) = '5050' \
           AND LTRIM(RTRIM(ISNULL(f.[KTRAEGER],''))) = '1' \
           AND LTRIM(RTRIM(ISNULL(f.[STORNIERT],''))) <> '1' \
           AND f.[PATNR] NOT LIKE '%[^0-9 ]%' \
           AND ISNULL(f.[{fd}],'') >= @P1 \
         GROUP BY SUBSTRING(ISNULL(f.[{fd}],''),1,6), {behandler}",
        fd = ident(&m.fakt_datum),
    );
    for r in &conn.rows(&sql_einzel, &[&cutoff]).await? {
        let Some(period) = period_from_ym(&get_str(r, "YM")) else { continue };
        out.push(RevenueRow {
            period,
            art: "bema".into(),
            gruppe: Some("PAR".into()),
            behandler: get_str(r, "BEHANDLER"),
            standort: None,
            honorar: round2(parse_amount(&get_str(r, "HONORAR"))),
            eigenlabor: None,
            fremdlabor: None,
            n_leistungen: i64::from(r.get::<i32, _>("N_LEIST").unwrap_or(0)),
            n_faelle: i64::from(r.get::<i32, _>("N_FAELLE").unwrap_or(0)),
        });
    }

    // 1b Sammel 0kp pro-rata (Gewicht = PARHIT.SUMTOTAL € je Plan/Periode).
    let sumtotal = sql_amount("h.SUMTOTAL");
    let fall = format!(
        "SELECT SUBSTRING(h.DTADATUM,5,2)+'.'+LEFT(h.DTADATUM,4) AS periode, \
                LTRIM(RTRIM(zp.LEBID)) AS LEBID_RAW, \
                CAST({sumtotal} AS float) AS gewicht \
         FROM PARHIT h \
         OUTER APPLY ( \
            SELECT TOP 1 zp.LEBID FROM ZPLAN zp \
            WHERE LTRIM(RTRIM(zp.PATNR)) = LTRIM(RTRIM(h.PATNR)) \
              AND LTRIM(RTRIM(zp.LFDPLAN)) = LTRIM(RTRIM(h.LFDPLAN)) \
         ) zp \
         WHERE h.DTADATUM <> '00000000' AND h.DTADATUM >= @P1"
    );
    out.extend(query_sammel_prorata(conn, cutoff, "0kp", "PAR", &fall).await?);
    Ok(out)
}

/// **ZE** (art=bema, gruppe=ZE): 2a Einzelrechnungen (FAKT RART 5060/6020, GKV) über
/// BILL→ZPLAN(→LEBID); 2b Sammelrechnung `0kz` pro-rata auf ZEHIT-Pläne (Gewicht =
/// Feld F4 = `SUBSTRING(ZE2,61,10)`).
async fn query_revenue_ze(
    conn: &mut Z1Connection,
    m: &ColumnMap,
    cutoff: &str,
) -> Result<Vec<RevenueRow>> {
    let mut out: Vec<RevenueRow> = Vec::new();

    // 2a Einzel: FAKT ⋈ BILL(→LFDHPLAN) ⋈ ZPLAN(→LEBID) → Behandler.
    let honorar = as_amount_str(&format!("SUM({})", sql_amount("f.[ZHON]")));
    let behandler = behandler_expr("zp.LEBID");
    let plan_join = PLAN_BEHANDLER_JOIN;
    let sql_einzel = format!(
        "SELECT SUBSTRING(ISNULL(f.[{fd}],''),1,6) AS YM, \
                {behandler} AS BEHANDLER, \
                {honorar} AS HONORAR, \
                CAST(COUNT(*) AS int) AS N_LEIST, \
                CAST(COUNT(DISTINCT LTRIM(RTRIM(f.[PATNR]))) AS int) AS N_FAELLE \
         FROM FAKT f \
         OUTER APPLY ( \
            SELECT TOP 1 b.LFDHPLAN FROM BILL b \
            WHERE LTRIM(RTRIM(b.PATNR)) = LTRIM(RTRIM(f.PATNR)) \
              AND LTRIM(RTRIM(b.GLFDFAKT)) = LTRIM(RTRIM(f.LFDFAKT)) \
         ) b \
         OUTER APPLY ( \
            SELECT TOP 1 zp.LEBID FROM ZPLAN zp \
            WHERE LTRIM(RTRIM(zp.PATNR)) = LTRIM(RTRIM(f.PATNR)) \
              AND LTRIM(RTRIM(zp.LFDPLAN)) = LTRIM(RTRIM(b.LFDHPLAN)) \
         ) zp \
         {plan_join} \
         WHERE LTRIM(RTRIM(ISNULL(f.[RART],''))) IN ('5060','6020') \
           AND LTRIM(RTRIM(ISNULL(f.[KTRAEGER],''))) = '1' \
           AND LTRIM(RTRIM(ISNULL(f.[STORNIERT],''))) <> '1' \
           AND f.[PATNR] NOT LIKE '%[^0-9 ]%' \
           AND ISNULL(f.[{fd}],'') >= @P1 \
         GROUP BY SUBSTRING(ISNULL(f.[{fd}],''),1,6), {behandler}",
        fd = ident(&m.fakt_datum),
    );
    for r in &conn.rows(&sql_einzel, &[&cutoff]).await? {
        let Some(period) = period_from_ym(&get_str(r, "YM")) else { continue };
        out.push(RevenueRow {
            period,
            art: "bema".into(),
            gruppe: Some("ZE".into()),
            behandler: get_str(r, "BEHANDLER"),
            standort: None,
            honorar: round2(parse_amount(&get_str(r, "HONORAR"))),
            eigenlabor: None,
            fremdlabor: None,
            n_leistungen: i64::from(r.get::<i32, _>("N_LEIST").unwrap_or(0)),
            n_faelle: i64::from(r.get::<i32, _>("N_FAELLE").unwrap_or(0)),
        });
    }

    // 2b Sammel 0kz pro-rata (Gewicht = ZEHIT Feld F4 = SUBSTRING(ZE2,61,10) €).
    let f4 = sql_amount("SUBSTRING(h.ZE2,61,10)");
    let fall = format!(
        "SELECT SUBSTRING(h.DTADATUM,5,2)+'.'+LEFT(h.DTADATUM,4) AS periode, \
                LTRIM(RTRIM(zp.LEBID)) AS LEBID_RAW, \
                CAST({f4} AS float) AS gewicht \
         FROM ZEHIT h \
         OUTER APPLY ( \
            SELECT TOP 1 zp.LEBID FROM ZPLAN zp \
            WHERE LTRIM(RTRIM(zp.PATNR)) = LTRIM(RTRIM(h.PATNR)) \
              AND LTRIM(RTRIM(zp.LFDPLAN)) = LTRIM(RTRIM(h.LFDPLAN)) \
         ) zp \
         WHERE h.DTADATUM <> '00000000' AND h.DTADATUM >= @P1"
    );
    out.extend(query_sammel_prorata(conn, cutoff, "0kz", "ZE", &fall).await?);
    Ok(out)
}

/// **KBR** (art=bema, gruppe=KBR): läuft rechnungsseitig unter RART 5060, ist aber
/// eine eigene Sparte. Nur Sammelrechnung `0kb` pro-rata auf KBRHIT-Pläne
/// (Gewicht = KBRHIT.SUMTOTAL €).
async fn query_revenue_kbr(
    conn: &mut Z1Connection,
    _m: &ColumnMap,
    cutoff: &str,
) -> Result<Vec<RevenueRow>> {
    let sumtotal = sql_amount("h.SUMTOTAL");
    let fall = format!(
        "SELECT SUBSTRING(h.DTADATUM,5,2)+'.'+LEFT(h.DTADATUM,4) AS periode, \
                LTRIM(RTRIM(zp.LEBID)) AS LEBID_RAW, \
                CAST({sumtotal} AS float) AS gewicht \
         FROM KBRHIT h \
         OUTER APPLY ( \
            SELECT TOP 1 zp.LEBID FROM ZPLAN zp \
            WHERE LTRIM(RTRIM(zp.PATNR)) = LTRIM(RTRIM(h.PATNR)) \
              AND LTRIM(RTRIM(zp.LFDPLAN)) = LTRIM(RTRIM(h.LFDPLAN)) \
         ) zp \
         WHERE h.DTADATUM <> '00000000' AND h.DTADATUM >= @P1"
    );
    query_sammel_prorata(conn, cutoff, "0kb", "KBR", &fall).await
}

/// Zahlungseingänge je Monat × Zahlart aus einer Quelle (`KONTO` oder `CASH`).
/// Zahlungseingänge aus CASH: je Monat × Zahlungsweg, stornierte ausgeschlossen.
async fn query_payments_cash(
    conn: &mut Z1Connection,
    m: &ColumnMap,
    cutoff: &str,
    acc: &mut BTreeMap<(String, String), (f64, i64)>,
) -> Result<()> {
    let summe = as_amount_str(&format!("SUM({})", sql_amount(&format!("[{}]", ident(&m.cash_betrag)))));
    let sql = format!(
        "SELECT SUBSTRING(ISNULL([{d}],''),1,6) AS YM, \
                LTRIM(RTRIM(ISNULL([{z}],''))) AS ART, \
                {summe} AS SUMME, CAST(COUNT(*) AS int) AS N \
         FROM CASH \
         WHERE ISNULL([{d}],'') >= @P1 \
           AND LTRIM(RTRIM(ISNULL([{s}],''))) IN ('', '0') \
         GROUP BY SUBSTRING(ISNULL([{d}],''),1,6), LTRIM(RTRIM(ISNULL([{z}],'')))",
        d = ident(&m.cash_datum),
        z = ident(&m.cash_zahlart),
        s = ident(&m.cash_storno),
    );
    let rows = conn.rows(&sql, &[&cutoff]).await?;
    for r in &rows {
        let Some(period) = period_from_ym(&get_str(r, "YM")) else {
            continue;
        };
        let art = map_art(&get_str(r, "ART"));
        let e = acc.entry((period, art)).or_insert((0.0, 0));
        e.0 += parse_amount(&get_str(r, "SUMME"));
        e.1 += i64::from(r.get::<i32, _>("N").unwrap_or(0));
    }
    Ok(())
}

/// Offene Forderungen aus FAKT ALLEIN: Rechnungen, die weder beglichen noch
/// storniert sind (KONTO ist der Kontenrahmen, keine Zahlungsquelle → nicht mehr
/// dagegen gerechnet). Je offene Rechnung eine Zeile mit Betrag + Datum, Bucket-
/// Zuordnung in Rust.
async fn query_ar_aging(
    conn: &mut Z1Connection,
    m: &ColumnMap,
    snapshot_date: &str,
    today: chrono::NaiveDate,
) -> Result<Vec<ArAgingRow>> {
    let amt_f = sql_amount(&format!("[{}]", ident(&m.fakt_betrag)));
    let offen_str = as_amount_str(&amt_f);
    let sql = format!(
        "SELECT ISNULL([{fd}],'') AS DAT, {offen_str} AS OFFEN \
         FROM FAKT \
         WHERE LTRIM(RTRIM(ISNULL([{offen}],''))) = '' \
           AND LTRIM(RTRIM(ISNULL([{storno}],''))) IN ('', '0') \
           AND {amt_f} > 0.005",
        fd = ident(&m.fakt_datum),
        offen = ident(&m.fakt_offen),
        storno = ident(&m.fakt_storno),
    );
    let rows = conn.rows(&sql, &[]).await?;
    let mut agg: BTreeMap<&'static str, (f64, i64)> = BTreeMap::new();
    for r in &rows {
        let offen = parse_amount(&get_str(r, "OFFEN"));
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
    // Liegengeblieben = erbrachte BEH-Leistung ohne Abrechnungsbezug (LFDPATBILL leer/0),
    // Betrag aus BEH.DMBETRAG. Kein Join, kein BILL-Subquery — der leere Abrechnungs-
    // Verweis IST das „nicht abgerechnet"-Signal.
    let offen = as_amount_str(&format!("SUM({})", sql_amount(&format!("b.[{}]", ident(&m.beh_betrag)))));
    let sql = format!(
        "SELECT LTRIM(RTRIM(ISNULL(b.[{beh}],''))) AS BEHANDLER, \
                {offen} AS OFFEN, CAST(COUNT(*) AS int) AS N, \
                MIN(NULLIF(LTRIM(RTRIM(ISNULL(b.[{d}],''))),'')) AS OLDEST \
         FROM BEH b \
         WHERE ISNULL(b.[{d}],'') >= @P1 \
           AND LTRIM(RTRIM(ISNULL(b.[{bl}],''))) IN ('', '0') \
         GROUP BY LTRIM(RTRIM(ISNULL(b.[{beh}],'')))",
        beh = ident(&m.beh_behandler),
        d = ident(&m.beh_datum),
        bl = ident(&m.beh_bill),
    );
    let rows = conn.rows(&sql, &[&cutoff]).await?;
    let mut out: Vec<OpenServicesRow> = rows
        .iter()
        .map(|r| OpenServicesRow {
            snapshot_date: snapshot_date.to_string(),
            behandler: get_str(r, "BEHANDLER"),
            offen_betrag: round2(parse_amount(&get_str(r, "OFFEN"))),
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

    // revenue: Privat/GOZ (BEH) + GKV je Behandler nach Sparte (KCH/PAR/ZE/KBR).
    // EINE Anforderungsliste über alle fünf Quellen — fehlt irgendeine benötigte
    // (Tabelle,Spalte), kommt revenue KOMPLETT nach pending_mappings. Spaltennamen
    // am ZMM (09.07.2026) per Schema-Discovery verifiziert.
    let req: Vec<(&str, &str)> = vec![
        // Privat/GOZ + KCH (BEH ⋈ GO/PUNKTWERTE), Behandler über BEH.LEBID.
        ("BEH", &map.beh_datum),
        ("BEH", &map.beh_art),
        ("BEH", &map.beh_behandler),
        ("BEH", &map.beh_betrag),
        ("BEH", &map.beh_patnr),
        ("BEH", "KYLSTNR"),
        ("BEH", "ANZAHL"),
        ("GO", "KYLSTNR"),
        ("GO", "GEBPKT"),
        ("GO", "ABDATUM"),
        ("GO", "BISDATUM"),
        ("GO", "PWLART"),
        ("PUNKTWERTE", "PWLART"),
        ("PUNKTWERTE", "PWWEST"),
        ("PUNKTWERTE", "ABDATUM"),
        ("LEB", "LEBID"),
        ("LEB", "PID"),
        ("PERSONAL", "PID"),
        ("PERSONAL", "KUERZEL"),
        // PAR/ZE/KBR: Einzel- + Sammelrechnungen, Behandler über den Plan (ZPLAN.LEBID).
        ("FAKT", &map.fakt_datum),
        ("FAKT", "ZHON"),
        ("FAKT", "RART"),
        ("FAKT", "KTRAEGER"),
        ("FAKT", "STORNIERT"),
        ("FAKT", "LFDFAKT"),
        ("FAKT", "LFDPATBILL"),
        ("FAKT", "PATNR"),
        ("FAKT", "BESCHREIBUNG"),
        ("BILL", "PATNR"),
        ("BILL", "GLFDFAKT"),
        ("BILL", "LFDHPLAN"),
        ("ZPLAN", "PATNR"),
        ("ZPLAN", "LFDPLAN"),
        ("ZPLAN", "LEBID"),
        ("PARHIT", "PATNR"),
        ("PARHIT", "LFDPATBILL"),
        ("PARHIT", "LFDPLAN"),
        ("PARHIT", "DTADATUM"),
        ("PARHIT", "SUMTOTAL"),
        ("KBRHIT", "PATNR"),
        ("KBRHIT", "LFDPLAN"),
        ("KBRHIT", "DTADATUM"),
        ("KBRHIT", "SUMTOTAL"),
        ("ZEHIT", "PATNR"),
        ("ZEHIT", "LFDPLAN"),
        ("ZEHIT", "DTADATUM"),
        ("ZEHIT", "ZE2"),
    ];
    let missing = disc.missing(&req);
    if missing.is_empty() {
        rows_scanned += count_beh_since(&mut conn, &map, &cutoff).await.unwrap_or(0);
        revenue = query_revenue(&mut conn, &map, &cutoff).await?;
    } else {
        pending.insert(
            "revenue".into(),
            pending_entry(
                missing,
                &disc,
                &[
                    "BEH", "GO", "PUNKTWERTE", "LEB", "PERSONAL", "FAKT", "BILL", "ZPLAN",
                    "PARHIT", "KBRHIT", "ZEHIT",
                ],
            ),
        );
    }

    // payments: CASH allein (KONTO ist der Kontenrahmen, keine Zahlungsquelle).
    let req_cash: Vec<(&str, &str)> = vec![
        ("CASH", &map.cash_datum),
        ("CASH", &map.cash_betrag),
        ("CASH", &map.cash_zahlart),
    ];
    let miss_cash = disc.missing(&req_cash);
    let mut pay_acc: BTreeMap<(String, String), (f64, i64)> = BTreeMap::new();
    if miss_cash.is_empty() {
        query_payments_cash(&mut conn, &map, &cutoff, &mut pay_acc).await?;
    } else {
        pending.insert("payments".into(), pending_entry(miss_cash, &disc, &["CASH"]));
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

    // ar_aging: FAKT allein (offene, nicht stornierte Rechnungen).
    let req: Vec<(&str, &str)> = vec![
        ("FAKT", &map.fakt_datum),
        ("FAKT", &map.fakt_betrag),
        ("FAKT", &map.fakt_offen),
        ("FAKT", &map.fakt_storno),
    ];
    let missing = disc.missing(&req);
    if missing.is_empty() {
        ar_aging = query_ar_aging(&mut conn, &map, &snapshot_date, today).await?;
    } else {
        pending.insert("ar_aging".into(), pending_entry(missing, &disc, &["FAKT"]));
    }

    // open_services: BEH allein, ohne Abrechnungsbezug (LFDPATBILL leer).
    let req: Vec<(&str, &str)> = vec![
        ("BEH", &map.beh_datum),
        ("BEH", &map.beh_behandler),
        ("BEH", &map.beh_betrag),
        ("BEH", &map.beh_bill),
    ];
    let missing = disc.missing(&req);
    if missing.is_empty() {
        open_services = query_open_services(&mut conn, &map, &cutoff, &snapshot_date).await?;
    } else {
        pending.insert("open_services".into(), pending_entry(missing, &disc, &["BEH"]));
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

/// Löscht den Tages-Marker → der nächste Tick führt den Sync SOFORT erneut aus
/// (statt bis morgen zu warten). Wird beim Speichern der Steuerungs-Config
/// aufgerufen, damit ein frisch aktivierter Sync oder ein geändertes Mapping
/// unmittelbar greift.
pub fn clear_last_run() {
    if let Ok(p) = paths::control_last_run_file() {
        let _ = std::fs::remove_file(p);
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
        // BEH hat DATUM/PATNR, aber nicht den Betrag DMBETRAG → Teil ausgelassen.
        let disc = disc_with(&[
            ("BEH", "PATNR"),
            ("BEH", "DATUM"),
            ("BEH", "GOART"),
            ("BEH", "LEBID"),
            ("BEH", "SONSTWAS"),
        ]);
        let m = ColumnMap::default();
        let req: Vec<(&str, &str)> = vec![
            ("BEH", &m.beh_patnr),
            ("BEH", &m.beh_datum),
            ("BEH", &m.beh_art),
            ("BEH", &m.beh_behandler),
            ("BEH", &m.beh_betrag),
        ];
        let missing = disc.missing(&req);
        assert_eq!(missing, vec!["BEH.DMBETRAG".to_string()]);

        let entry = pending_entry(missing, &disc, &["BEH"]);
        assert_eq!(entry["missing"][0], "BEH.DMBETRAG");
        let avail: Vec<String> = entry["available"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(avail.contains(&"BEH.PATNR".to_string()));
        assert!(avail.contains(&"BEH.SONSTWAS".to_string()));
        assert!(avail.contains(&"BEH.GOART".to_string()));

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
    fn periode_aus_mm_jjjj() {
        assert_eq!(period_from_mmjjjj("07.2026"), Some("2026-07-01".into()));
        assert_eq!(period_from_mmjjjj(" 12.2025 "), Some("2025-12-01".into()));
        assert_eq!(period_from_mmjjjj("13.2026"), None); // Monat 13
        assert_eq!(period_from_mmjjjj("00.2026"), None); // Monat 0
        assert_eq!(period_from_mmjjjj("7.2026"), None); // fehlendes Padding/Trenner
        assert_eq!(period_from_mmjjjj("2026-07"), None); // falscher Trenner
        assert_eq!(period_from_mmjjjj(""), None);
        assert_eq!(period_from_mmjjjj("aa.bbbb"), None);
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
        // Cent-Transform: 'e' + Leerzeichen raus, Ziffern als bigint (Cent) / 100.
        let e = sql_amount("[BETRAG]");
        assert!(e.contains("bigint"), "{e}");
        assert!(e.contains("/ 100.0"), "{e}");
        assert!(e.contains("REPLACE") && e.contains("'e'"), "{e}");
        assert_eq!(ident("EINZELBETRAG"), "EINZELBETRAG");
        assert_eq!(ident("X]; DROP TABLE PAT;--"), "XDROPTABLEPAT");
    }

    #[test]
    fn parse_amount_liest_dezimal_string() {
        // as_amount_str liefert SQL-seitig einen Dezimal-String ("64.26"); Rust parst ihn.
        assert_eq!(parse_amount("64.26"), 64.26);
        assert_eq!(parse_amount("1234.56"), 1234.56);
        assert_eq!(parse_amount("0.00"), 0.0);
        assert_eq!(parse_amount(""), 0.0);
        assert_eq!(parse_amount("-12.50"), -12.5);
    }
}
