//! Z1-Adapter der Potenzialanalyse: füllt [`crate::analysis::AnalysisInputs`]
//! read-only aus der Z1-SQL-DB.
//!
//! Alle Queries wurden am ZMM-Piloten live entwickelt und verifiziert
//! (2026-07-13). Geldformat/Fallstricke: `docs/Z1-BILLING.md` (Cent-Format
//! `e NNN`, PATNR rechtsbündig, TRY_CAST für Datenrauschen). Jede Kennzahl
//! wird einzeln und fehlertolerant erhoben — fällt eine Query aus (Schema-
//! Abweichung bei anderer Z1-Version), fehlt nur dieser Befund im Report.

use crate::analysis::{
    AnalysisInputs, FactoringSmall, ParStats, PlanLeakage, ProphylaxeStats, Receivables,
};
use crate::z1db::control::{as_amount_str, parse_amount};
use crate::z1db::Z1Connection;
use chrono::{Datelike, Months, NaiveDate};
use tracing::debug;

fn ymd(d: NaiveDate) -> String {
    d.format("%Y%m%d").to_string()
}

/// SQL-Fragment: Z1-Betragsfeld (`e NNN` = Cent) → Euro-`bigint`-Cents.
fn cents(col: &str) -> String {
    format!("CAST(NULLIF(REPLACE(REPLACE({col},'e',''),' ',''),'') AS bigint)")
}

/// Erhebt alle verfügbaren Kennzahlen. Einzelne Ausfälle sind nicht fatal.
pub async fn collect_inputs(conn: &mut Z1Connection, today: NaiveDate) -> AnalysisInputs {
    let stichtag = ymd(today);
    let m12 = ymd(today - Months::new(12));
    let m24 = ymd(today - Months::new(24));
    let m36 = ymd(today - Months::new(36));
    let m9 = ymd(today - Months::new(9));
    let prev_year = today.year() - 1;

    let mut inputs = AnalysisInputs {
        pvs: "Z1".into(),
        stichtag: stichtag.clone(),
        ..Default::default()
    };

    // Aktive Patienten (12 Monate) — Basisgröße.
    inputs.active_patients_12m = scalar_i64(
        conn,
        &format!(
            "SELECT COUNT(DISTINCT LTRIM(RTRIM(PATNR))) FROM BEH WHERE DATUM>='{m12}' AND DATUM<='{stichtag}'"
        ),
    )
    .await;

    // Faktura des letzten vollen Kalenderjahres. (Beträge grundsätzlich als
    // String lesen — tiberius liefert SUM(float) sonst als 0, s. control.rs.)
    inputs.revenue_last_year_eur = scalar_f64(
        conn,
        &format!(
            "SELECT {} FROM FAKT WHERE STORNIERT<>'1' AND FAKTDATUM>='{prev_year}0101' AND FAKTDATUM<='{prev_year}1231'",
            as_amount_str(&format!("SUM({})/100.0", cents("BETRAG")))
        ),
    )
    .await;

    // Wiederkehrer: aktiv im Fenster 12–24 Monate zurück, erneut gesehen in 0–12.
    inputs.returning_base = scalar_i64(
        conn,
        &format!(
            "SELECT COUNT(DISTINCT LTRIM(RTRIM(PATNR))) FROM BEH WHERE DATUM>='{m24}' AND DATUM<'{m12}'"
        ),
    )
    .await;
    inputs.returning_seen_again = scalar_i64(
        conn,
        &format!(
            "SELECT COUNT(*) FROM (SELECT DISTINCT LTRIM(RTRIM(PATNR)) AS p FROM BEH WHERE DATUM>='{m24}' AND DATUM<'{m12}') a \
             WHERE EXISTS (SELECT 1 FROM BEH b WHERE LTRIM(RTRIM(b.PATNR))=a.p AND b.DATUM>='{m12}' AND b.DATUM<='{stichtag}')"
        ),
    )
    .await;

    // Abwesenheits-Kohorten (lebende, nicht gesperrte Patienten).
    for (col, from, to) in [
        ("inactive_12_24m", m24.clone(), m12.clone()),
        ("inactive_24_36m", m36.clone(), m24.clone()),
    ] {
        let v = scalar_i64(
            conn,
            &format!(
                "SELECT COUNT(*) FROM (SELECT LTRIM(RTRIM(PATNR)) AS p, MAX(DATUM) AS letzter FROM BEH GROUP BY LTRIM(RTRIM(PATNR))) l \
                 JOIN PAT pa ON LTRIM(RTRIM(pa.PATNR))=l.p \
                 WHERE ISNULL(pa.VERSTORBENAM,'')='' AND ISNULL(pa.GESPERRT,'')<>'1' AND l.letzter>='{from}' AND l.letzter<'{to}'"
            ),
        )
        .await;
        match col {
            "inactive_12_24m" => inputs.inactive_12_24m = v,
            _ => inputs.inactive_24_36m = v,
        }
    }

    // Prophylaxe (GOZ 1040 = PZR) der letzten 12 Monate.
    inputs.prophylaxe = prophylaxe(conn, &m12, &stichtag, inputs.active_patients_12m).await;

    // Verfallende genehmigte eHKPs (mit Euro aus dem EEBZ0-XML).
    inputs.plan_leakage = plan_leakage(conn, today).await;

    // PAR-/UPT-Strecken.
    inputs.par = par_stats(conn, &m9, &stichtag, today).await;

    // Forderungslage: Ausbuchungen (Ø der letzten 3 vollen Jahre) + offene Direktforderungen.
    inputs.receivables = receivables(conn, prev_year).await;

    // Erbrachte, nicht fakturierte Privatleistungen (Bestand, älter 3 Monate).
    let m3 = ymd(today - Months::new(3));
    inputs.unbilled_private_eur = scalar_f64(
        conn,
        &format!(
            "SELECT {} FROM BEH WHERE GOART IN ('q','2','3','4','7') \
             AND LTRIM(RTRIM(ISNULL(LFDPATBILL,'')))='' AND LTRIM(RTRIM(ISNULL(DMBETRAG,'')))<>'' \
             AND DATUM>='20240101' AND DATUM<'{m3}'",
            as_amount_str(&format!("ISNULL(SUM({})/100.0, 0)", cents("DMBETRAG")))
        ),
    )
    .await;

    // Kleinbetrags-Factoring: RZ-Rechnungen < 200 € der letzten 12 Monate.
    inputs.factoring_small = factoring_small(conn, &m12, &stichtag).await;

    // Dokumentierte No-Shows (Kartei, Untergrenze).
    inputs.no_shows_documented_12m = scalar_i64(
        conn,
        &format!(
            "SELECT COUNT(*) FROM BEH WHERE DATUM>='{m12}' AND (BEHTEXT LIKE '%nicht erschienen%' \
             OR BEHTEXT LIKE '%unentschuldigt%' OR BEHTEXT LIKE '%nicht abgesagt%')"
        ),
    )
    .await;

    inputs
}

async fn prophylaxe(
    conn: &mut Z1Connection,
    m12: &str,
    stichtag: &str,
    active: Option<i64>,
) -> Option<ProphylaxeStats> {
    let freq = conn
        .rows(
            &format!(
                "WITH pzr AS (SELECT LTRIM(RTRIM(PATNR)) AS p, COUNT(*) AS anz FROM BEH \
                 WHERE LTRIM(RTRIM(LSTNR))='1040' AND DATUM>='{m12}' AND DATUM<='{stichtag}' GROUP BY LTRIM(RTRIM(PATNR))) \
                 SELECT SUM(CASE WHEN anz=1 THEN 1 ELSE 0 END), SUM(CASE WHEN anz=2 THEN 1 ELSE 0 END), \
                        SUM(CASE WHEN anz>=3 THEN 1 ELSE 0 END), COUNT(*) FROM pzr"
            ),
            &[],
        )
        .await
        .ok()?;
    let row = freq.first()?;
    let (f1, f2, f3, with_pzr) = (
        row.get::<i32, _>(0).unwrap_or(0) as i64,
        row.get::<i32, _>(1).unwrap_or(0) as i64,
        row.get::<i32, _>(2).unwrap_or(0) as i64,
        row.get::<i32, _>(3).unwrap_or(0) as i64,
    );
    let avg_expr = as_amount_str(
        "AVG(CAST(NULLIF(REPLACE(REPLACE(DMBETRAG,'e',''),' ',''),'') AS float))/100.0",
    );
    let stats = conn
        .rows(
            &format!(
                "SELECT COUNT(*), {avg_expr} FROM BEH WHERE LTRIM(RTRIM(LSTNR))='1040' \
                 AND DATUM>='{m12}' AND DATUM<='{stichtag}' AND LTRIM(RTRIM(ISNULL(DMBETRAG,'')))<>''"
            ),
            &[],
        )
        .await
        .ok()?;
    let srow = stats.first()?;
    let services = srow.get::<i32, _>(0).unwrap_or(0) as i64;
    let avg = srow.get::<&str, _>(1).map(parse_amount).unwrap_or(0.0);
    let active = active?;
    Some(ProphylaxeStats {
        active_patients: active,
        without_pzr: (active - with_pzr).max(0),
        freq_1x: f1,
        freq_2x: f2,
        freq_3plus: f3,
        avg_price_eur: avg,
        services_12m: services,
    })
}

/// Genehmigte, nicht eingegliederte eHKPs über der 6-Monats-Frist — Euro-Wert
/// direkt aus dem EEBZ0-XML in `FILEPOOL` (String-Extraktion, verifiziert).
async fn plan_leakage(conn: &mut Z1Connection, today: NaiveDate) -> Option<PlanLeakage> {
    let cutoff = ymd(today - Months::new(6));
    let window_start = "20240101";
    let window_years = (today - NaiveDate::from_ymd_opt(2024, 1, 1)?).num_days() as f64 / 365.25;
    let rows = conn
        .rows(
            &format!(
                "WITH dec AS (SELECT LTRIM(RTRIM(PATNR)) AS patnr, LTRIM(RTRIM(LFDPLAN)) AS lfdplan, MAX(ERHALTDATUM) AS gen_dat \
                   FROM EBZ WHERE DOKART='3' AND ZUGESTELLT='1' GROUP BY LTRIM(RTRIM(PATNR)), LTRIM(RTRIM(LFDPLAN))), \
                 eing AS (SELECT DISTINCT LTRIM(RTRIM(PATNR)) AS patnr, LTRIM(RTRIM(LFDPLAN)) AS lfdplan FROM ZEHIT WHERE ISNULL(EINGLIEDERUNGSDATUM,'')<>''), \
                 kand AS (SELECT LTRIM(RTRIM(z.PATNR)) AS patnr, ISNULL(z.DEAKTIVIERTDATUM,'') AS deakt, \
                          LEFT(LTRIM(RTRIM(z.ANTRAGSNUMMER)), CHARINDEX(' ', LTRIM(RTRIM(z.ANTRAGSNUMMER))+' ')-1) AS token \
                   FROM ZPLAN z JOIN dec d ON d.patnr=LTRIM(RTRIM(z.PATNR)) AND d.lfdplan=LTRIM(RTRIM(z.LFDPLAN)) \
                   WHERE LTRIM(RTRIM(z.PLANART))='3' AND ISNULL(z.KZVABRDATUM,'')='' AND d.gen_dat<='{cutoff}' AND d.gen_dat>='{window_start}' \
                     AND NOT EXISTS (SELECT 1 FROM eing e WHERE e.patnr=LTRIM(RTRIM(z.PATNR)) AND e.lfdplan=LTRIM(RTRIM(z.LFDPLAN)))), \
                 mitxml AS (SELECT k.deakt, x.wert FROM kand k \
                   OUTER APPLY (SELECT TOP 1 SUBSTRING(CAST(f.FILEDATA AS varchar(max)), \
                       CHARINDEX('<zer:Behandlungskosten_insgesamt>', CAST(f.FILEDATA AS varchar(max)))+33, \
                       CHARINDEX('</zer:Behandlungskosten_insgesamt>', CAST(f.FILEDATA AS varchar(max))) \
                         - CHARINDEX('<zer:Behandlungskosten_insgesamt>', CAST(f.FILEDATA AS varchar(max)))-33) AS wert \
                     FROM FILEPOOL f WHERE f.FILENAME LIKE 'EEBZ0_'+k.token+'%.xml' \
                       AND CHARINDEX('<zer:Behandlungskosten_insgesamt>', CAST(f.FILEDATA AS varchar(max)))>0 \
                     ORDER BY f.FILENAME DESC) x) \
                 SELECT CASE WHEN deakt<>'' THEN 1 ELSE 0 END, COUNT(*), \
                        {sum_expr} \
                 FROM mitxml GROUP BY CASE WHEN deakt<>'' THEN 1 ELSE 0 END",
                sum_expr = as_amount_str(
                    "ISNULL(SUM(TRY_CAST(REPLACE(REPLACE(wert,'.',''),',','.') AS float)),0)"
                )
            ),
            &[],
        )
        .await
        .ok()?;
    let mut out = PlanLeakage { window_years, ..Default::default() };
    for r in &rows {
        let deakt = r.get::<i32, _>(0).unwrap_or(0) == 1;
        let n = r.get::<i32, _>(1).unwrap_or(0) as i64;
        let sum = r.get::<&str, _>(2).map(parse_amount);
        if deakt {
            out.deactivated_cases = n;
            out.deactivated_value_eur = sum;
        } else {
            out.expired_open_cases = n;
            out.expired_open_value_eur = sum;
        }
    }
    Some(out)
}

async fn par_stats(
    conn: &mut Z1Connection,
    m9: &str,
    stichtag: &str,
    today: NaiveDate,
) -> Option<ParStats> {
    let approval_cutoff = ymd(today - Months::new(3)); // frisch genehmigten Zeit lassen
    let plans = conn
        .rows(
            &format!(
                "WITH dec AS (SELECT LTRIM(RTRIM(PATNR)) AS patnr, LTRIM(RTRIM(LFDPLAN)) AS lfdplan, MAX(ERHALTDATUM) AS gen_dat \
                   FROM EBZ WHERE DOKART='3' AND ZUGESTELLT='1' GROUP BY LTRIM(RTRIM(PATNR)), LTRIM(RTRIM(LFDPLAN))), \
                 par AS (SELECT LTRIM(RTRIM(z.PATNR)) AS patnr FROM ZPLAN z JOIN dec d \
                   ON d.patnr=LTRIM(RTRIM(z.PATNR)) AND d.lfdplan=LTRIM(RTRIM(z.LFDPLAN)) \
                   WHERE LTRIM(RTRIM(z.PLANART))='4' AND d.gen_dat>='20240101' AND d.gen_dat<='{approval_cutoff}'), \
                 begonnen AS (SELECT DISTINCT LTRIM(RTRIM(PATNR)) AS patnr FROM BEH \
                   WHERE LTRIM(RTRIM(LSTNR)) IN ('atg','aita','aitb') AND DATUM>='20240101') \
                 SELECT (SELECT COUNT(*) FROM par), \
                        (SELECT COUNT(*) FROM par p WHERE NOT EXISTS (SELECT 1 FROM begonnen b WHERE b.patnr=p.patnr))"
            ),
            &[],
        )
        .await
        .ok()?;
    let prow = plans.first()?;
    let (approved, never_started) = (
        prow.get::<i32, _>(0).unwrap_or(0) as i64,
        prow.get::<i32, _>(1).unwrap_or(0) as i64,
    );
    let upt = conn
        .rows(
            &format!(
                "WITH strecken AS (SELECT DISTINCT LTRIM(RTRIM(PATNR)) AS patnr FROM BEH \
                   WHERE LTRIM(RTRIM(LSTNR)) IN ('beva','aita','aitb') AND DATUM>='20240101' AND DATUM<'{m9}'), \
                 aktuell AS (SELECT DISTINCT LTRIM(RTRIM(PATNR)) AS patnr FROM BEH \
                   WHERE LTRIM(RTRIM(LSTNR)) LIKE 'upt%' AND DATUM>='{m9}' AND DATUM<='{stichtag}') \
                 SELECT (SELECT COUNT(*) FROM strecken), \
                        (SELECT COUNT(*) FROM strecken s WHERE NOT EXISTS (SELECT 1 FROM aktuell a WHERE a.patnr=s.patnr))"
            ),
            &[],
        )
        .await
        .ok()?;
    let urow = upt.first()?;
    Some(ParStats {
        approved_plans: approved,
        never_started,
        upt_expected: urow.get::<i32, _>(0).unwrap_or(0) as i64,
        upt_broken: urow.get::<i32, _>(1).unwrap_or(0) as i64,
    })
}

async fn receivables(conn: &mut Z1Connection, prev_year: i32) -> Option<Receivables> {
    let from = prev_year - 2;
    let written_off = scalar_f64(
        conn,
        &format!(
            "SELECT {} FROM FAKT WHERE STORNIERT<>'1' \
             AND FAKTDATUM>='{from}0101' AND FAKTDATUM<='{prev_year}1231'",
            as_amount_str(&format!("ISNULL(SUM({})/100.0, 0)/3.0", cents("AUSGEBUCHT")))
        ),
    )
    .await?;
    // Offene Direktforderungen: nur selbst gestellte Privatrechnungen (KTRAEGER 'a')
    // — die RZ-gefactorten (z) sind bezahlt, Z1s „beglichen" dort Buchungsartefakt.
    let open_direct = scalar_f64(
        conn,
        &format!(
            "SELECT {} FROM FAKT WHERE STORNIERT<>'1' AND KTRAEGER='a'",
            as_amount_str(&format!(
                "ISNULL(SUM({b} - ISNULL({p},0) - ISNULL({a},0))/100.0, 0)",
                b = cents("BETRAG"),
                p = cents("BEGLICHEN"),
                a = cents("AUSGEBUCHT")
            ))
        ),
    )
    .await?;
    Some(Receivables { written_off_avg_year_eur: written_off, open_direct_eur: open_direct.max(0.0) })
}

async fn factoring_small(
    conn: &mut Z1Connection,
    m12: &str,
    stichtag: &str,
) -> Option<FactoringSmall> {
    let rows = conn
        .rows(
            &format!(
                "SELECT COUNT(*), {sum_expr} FROM FAKT \
                 WHERE STORNIERT<>'1' AND KTRAEGER='z' AND FAKTDATUM>='{m12}' AND FAKTDATUM<='{stichtag}' \
                 AND {b} < 20000",
                b = cents("BETRAG"),
                sum_expr = as_amount_str(&format!("ISNULL(SUM({})/100.0, 0)", cents("BETRAG")))
            ),
            &[],
        )
        .await
        .ok()?;
    let row = rows.first()?;
    let n = row.get::<i32, _>(0).unwrap_or(0) as i64;
    if n == 0 {
        return None; // Praxis nutzt kein RZ → Befund entfällt
    }
    Some(FactoringSmall {
        invoices_12m: n,
        volume_12m_eur: row.get::<&str, _>(1).map(parse_amount).unwrap_or(0.0),
        threshold_eur: 200.0,
    })
}

// ── kleine, fehlertolerante Skalar-Helfer ────────────────────────────────────

async fn scalar_i64(conn: &mut Z1Connection, sql: &str) -> Option<i64> {
    match conn.rows(sql, &[]).await {
        Ok(rows) => rows.first().and_then(|r| r.get::<i32, _>(0)).map(i64::from),
        Err(e) => {
            debug!(error=%e, "Analyse-Query fehlgeschlagen (Kennzahl entfällt)");
            None
        }
    }
}

/// Liest einen (per [`as_amount_str`] als varchar gelieferten) Betrag.
async fn scalar_f64(conn: &mut Z1Connection, sql: &str) -> Option<f64> {
    match conn.rows(sql, &[]).await {
        Ok(rows) => rows.first().and_then(|r| r.get::<&str, _>(0)).map(parse_amount),
        Err(e) => {
            debug!(error=%e, "Analyse-Query fehlgeschlagen (Kennzahl entfällt)");
            None
        }
    }
}
