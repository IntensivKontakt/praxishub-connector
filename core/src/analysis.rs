//! Potenzialanalyse — PVS-agnostisch.
//!
//! Quantifiziert aus PVS-Rohzahlen die ökonomischen Hebel einer Praxis
//! („wenn man X ändert, bringt das Y €/Jahr") — als **Einmal-Report für den
//! Sales-Prozess** (lokal, ohne Cloud: `--potenzialanalyse`) und als
//! **laufender Feed** fürs Praxis-Steuerungs-Dashboard (über den Control-Sync).
//!
//! Architektur: Jeder PVS-Adapter (Z1: [`crate::z1db::analysis`]; künftig
//! charly/DS-Win/Dampsoft) füllt NUR die neutrale Rohzahlen-Struktur
//! [`AnalysisInputs`] — alle Felder optional, ein Adapter liefert, was sein
//! PVS hergibt. Die Bewertung ([`evaluate`]) und das Rendering
//! ([`render_html`]) sind rein und PVS-frei. **Bewusst werden auch unauffällige
//! Ergebnisse berichtet** („Abrechnung läuft sauber") — der Report soll ehrlich
//! sein, nicht alarmistisch.
//!
//! Kalibriert an der Live-Analyse des Z1-Piloten (2026-07-13, siehe
//! Memory/`docs`): Annahme-Konstanten unten sind konservativ gewählt und
//! im Report ausgewiesen.

use serde::{Deserialize, Serialize};

// ── Annahmen (konservativ; im Report ausgewiesen) ────────────────────────────

/// Anteil der 1×-Prophylaxe-Patienten, die auf 2×/Jahr zu heben sind.
const PZR_UPGRADE_RATE: f64 = 0.4;
/// Anteil der aktiven Patienten ohne Prophylaxe, die für 1×/Jahr zu gewinnen sind.
const PZR_WIN_RATE: f64 = 0.2;
/// Reaktivierungsquote für 12–24 Monate abwesende Patienten.
const REACT_RATE_12_24: f64 = 0.15;
/// Reaktivierungsquote für 24–36 Monate abwesende Patienten.
const REACT_RATE_24_36: f64 = 0.05;
/// Rettbare Quote verfallender genehmigter Pläne (min/max).
const PLAN_RESCUE_MIN: f64 = 0.3;
const PLAN_RESCUE_MAX: f64 = 0.4;
/// Wert einer nie begonnenen PAR-Strecke (AIT-Phase, €).
const PAR_START_VALUE: f64 = 600.0;
/// Entgangenes UPT-Honorar je abgerissener Strecke (€/Jahr).
const UPT_VALUE_PER_YEAR: f64 = 250.0;
/// Netto-Ersparnis, wenn Kleinbeträge nicht mehr gefactort werden
/// (Factoring-Gebühr minus eigene Zahlungswege), min/max vom Volumen.
const FACTORING_SAVE_MIN: f64 = 0.01;
const FACTORING_SAVE_MAX: f64 = 0.03;

// ── Rohzahlen (vom PVS-Adapter geliefert) ────────────────────────────────────

/// Prophylaxe-Kennzahlen der letzten 12 Monate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProphylaxeStats {
    /// Aktive Patienten (letzte 12 Monate) insgesamt.
    pub active_patients: i64,
    /// … davon ohne jede Prophylaxe-Leistung.
    pub without_pzr: i64,
    /// Patienten mit genau 1 / genau 2 / 3+ Prophylaxen.
    pub freq_1x: i64,
    pub freq_2x: i64,
    pub freq_3plus: i64,
    /// Durchschnittspreis einer Prophylaxe (€).
    pub avg_price_eur: f64,
    /// Prophylaxe-Leistungen gesamt (12 Monate).
    pub services_12m: i64,
}

/// Genehmigte, aber nie umgesetzte Behandlungspläne (HKP o. Ä.).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanLeakage {
    /// Fälle: genehmigt, nicht eingegliedert, über Gültigkeitsfrist.
    pub expired_open_cases: i64,
    /// Summierter Behandlungswert dieser Fälle (€), falls ermittelbar.
    pub expired_open_value_eur: Option<f64>,
    /// Bewusst deaktivierte genehmigte Fälle (Kontext, kein Hebel).
    pub deactivated_cases: i64,
    pub deactivated_value_eur: Option<f64>,
    /// Beobachtungszeitraum in Jahren (zur €/Jahr-Normierung).
    pub window_years: f64,
}

/// PAR-/UPT-Strecken (falls das PVS sie abbildet).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParStats {
    pub approved_plans: i64,
    pub never_started: i64,
    pub upt_expected: i64,
    pub upt_broken: i64,
}

/// Forderungslage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Receivables {
    /// Ausgebuchte (verlorene) Forderungen, Ø €/Jahr.
    pub written_off_avg_year_eur: f64,
    /// Aktuell offene, selbst einzuziehende Forderungen (€, ohne Factoring).
    pub open_direct_eur: f64,
}

/// Kleinbetrags-Factoring (falls die Praxis ein RZ nutzt).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FactoringSmall {
    /// Rechnungen unter der Kleinbetragsgrenze, letzte 12 Monate.
    pub invoices_12m: i64,
    /// Volumen dieser Rechnungen (€, 12 Monate).
    pub volume_12m_eur: f64,
    /// Kleinbetragsgrenze (€), nur zur Anzeige.
    pub threshold_eur: f64,
}

/// Neutrale Rohzahlen — der komplette PVS-Vertrag der Potenzialanalyse.
/// Jeder Adapter füllt, was sein System hergibt; `None` = im Report als
/// „nicht ermittelbar" ausgewiesen.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalysisInputs {
    /// PVS-Name (z. B. "Z1", "charly", "DS-Win").
    pub pvs: String,
    /// Stichtag `JJJJMMTT`.
    pub stichtag: String,
    /// Aktive Patienten (Leistung in den letzten 12 Monaten).
    pub active_patients_12m: Option<i64>,
    /// Gesamt-Faktura des letzten vollen Kalenderjahres (€).
    pub revenue_last_year_eur: Option<f64>,
    /// Wiederkehrer: Patienten aktiv im Vorjahresfenster …
    pub returning_base: Option<i64>,
    /// … davon im letzten 12-Monats-Fenster erneut gesehen.
    pub returning_seen_again: Option<i64>,
    /// Abwesende Patienten (nicht verstorben/gesperrt) nach letztem Besuch.
    pub inactive_12_24m: Option<i64>,
    pub inactive_24_36m: Option<i64>,
    pub inactive_over_36m: Option<i64>,
    pub prophylaxe: Option<ProphylaxeStats>,
    pub plan_leakage: Option<PlanLeakage>,
    pub par: Option<ParStats>,
    pub receivables: Option<Receivables>,
    /// Erbrachte, nie fakturierte Privatleistungen (€, Bestand).
    pub unbilled_private_eur: Option<f64>,
    pub factoring_small: Option<FactoringSmall>,
    /// Im PVS dokumentierte No-Shows (12 Monate) — meist Untergrenze.
    pub no_shows_documented_12m: Option<i64>,
}

// ── Bewertung ────────────────────────────────────────────────────────────────

/// Befund-Kategorie: Chance (Hebel mit €), solide (läuft gut — bewusst
/// berichten!), Info (nicht quantifizierbar / Datenlücke).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Chance,
    Solide,
    Info,
}

/// Ein bewerteter Befund.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Stabiler Schlüssel (z. B. "prophylaxe", "plan_leakage").
    pub key: String,
    pub title: String,
    pub verdict: Verdict,
    /// Ein-Satz-Kernaussage.
    pub summary: String,
    /// Datenbelege (je Zeile ein Fakt aus dem PVS).
    pub evidence: Vec<String>,
    /// Jahres-Potenzial in € (konservativ, min/max), nur bei `Chance`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub potential_eur_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub potential_eur_max: Option<f64>,
    /// Empfohlene Maßnahme (und wie Praxishub sie abdeckt).
    pub recommendation: String,
}

/// Gesamtreport = Rohzahlen + bewertete Befunde + Summen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PotentialReport {
    pub pvs: String,
    pub stichtag: String,
    pub inputs: AnalysisInputs,
    pub findings: Vec<Finding>,
    pub total_potential_eur_min: f64,
    pub total_potential_eur_max: f64,
}

fn eur(v: f64) -> String {
    // Deutsche Tausendertrennung, ganze Euro (Report-Anzeige).
    let n = v.round() as i64;
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    format!("{}{} €", if n < 0 { "-" } else { "" }, out)
}

fn pct(part: i64, total: i64) -> f64 {
    if total <= 0 {
        0.0
    } else {
        (part as f64) * 100.0 / (total as f64)
    }
}

/// Bewertet die Rohzahlen zu einem Report. Rein und PVS-frei — testbar.
pub fn evaluate(inputs: &AnalysisInputs) -> PotentialReport {
    let mut findings: Vec<Finding> = Vec::new();
    let rev_per_patient = match (inputs.revenue_last_year_eur, inputs.active_patients_12m) {
        (Some(r), Some(a)) if a > 0 => Some(r / a as f64),
        _ => None,
    };

    // 1) Prophylaxe/Recall.
    if let Some(p) = &inputs.prophylaxe {
        let upgrade = p.freq_1x as f64 * PZR_UPGRADE_RATE * p.avg_price_eur;
        let win = p.without_pzr as f64 * PZR_WIN_RATE * p.avg_price_eur;
        let potential = upgrade + win;
        findings.push(Finding {
            key: "prophylaxe".into(),
            title: "Prophylaxe-Frequenz & -Abdeckung".into(),
            verdict: if potential > 10_000.0 { Verdict::Chance } else { Verdict::Solide },
            summary: format!(
                "{:.0} % der aktiven Patienten ohne Prophylaxe; nur {:.0} % der Prophylaxe-Patienten kommen 2×/Jahr.",
                pct(p.without_pzr, p.active_patients),
                pct(p.freq_2x + p.freq_3plus, p.freq_1x + p.freq_2x + p.freq_3plus)
            ),
            evidence: vec![
                format!("{} von {} aktiven Patienten ohne jede Prophylaxe", p.without_pzr, p.active_patients),
                format!("{} Patienten nur 1×, {} 2×, {} 3+×", p.freq_1x, p.freq_2x, p.freq_3plus),
                format!("Ø-Preis {} · {} Leistungen in 12 Monaten", eur(p.avg_price_eur), p.services_12m),
            ],
            potential_eur_min: Some(potential * 0.6),
            potential_eur_max: Some(potential),
            recommendation: "Automatisiertes Recall mit Online-Terminlink (2×-Intervall aktiv anbieten); Kapazität im Prophylaxe-Plan prüfen.".into(),
        });
    }

    // 2) Reaktivierung abwesender Patienten.
    if let (Some(i12), Some(i24)) = (inputs.inactive_12_24m, inputs.inactive_24_36m) {
        if let Some(rpp) = rev_per_patient {
            let potential = i12 as f64 * REACT_RATE_12_24 * rpp + i24 as f64 * REACT_RATE_24_36 * rpp;
            findings.push(Finding {
                key: "reaktivierung".into(),
                title: "Abwesende Patienten reaktivieren".into(),
                verdict: if potential > 10_000.0 { Verdict::Chance } else { Verdict::Solide },
                summary: format!(
                    "{} Patienten seit 12–24 Monaten, {} seit 24–36 Monaten nicht mehr da.",
                    i12, i24
                ),
                evidence: vec![
                    format!("Ø-Umsatz je aktivem Patient: {}", eur(rpp)),
                    format!("Rechnung: {:.0} % der 12–24er + {:.0} % der 24–36er zurückgewinnen",
                        REACT_RATE_12_24 * 100.0, REACT_RATE_24_36 * 100.0),
                    "Überschneidet sich teilweise mit dem Prophylaxe-Hebel — nicht voll addieren.".into(),
                ],
                potential_eur_min: Some(potential * 0.5),
                potential_eur_max: Some(potential),
                recommendation: "Reaktivierungskampagne (gestaffelt nach Abwesenheitsdauer) mit direktem Online-Termin.".into(),
            });
        }
    }

    // 3) Wiederkehrer-Quote (Patientenbindung).
    if let (Some(base), Some(again)) = (inputs.returning_base, inputs.returning_seen_again) {
        let quote = pct(again, base);
        findings.push(Finding {
            key: "wiederkehrer".into(),
            title: "Patientenbindung (Wiederkehrer-Quote)".into(),
            verdict: if quote < 65.0 { Verdict::Chance } else { Verdict::Solide },
            summary: format!("{quote:.0} % der Patienten des Vorjahres waren in den letzten 12 Monaten wieder da."),
            evidence: vec![format!("{again} von {base} Vorjahres-Patienten erneut gesehen")],
            potential_eur_min: None,
            potential_eur_max: None,
            recommendation: if quote < 65.0 {
                "Bindung unter Benchmark (~70–80 %) — Recall + Terminerinnerungen priorisieren.".into()
            } else {
                "Quote im gesunden Bereich — mit Recall-Automatisierung halten.".into()
            },
        });
    }

    // 4) Verfallende genehmigte Pläne.
    if let Some(pl) = &inputs.plan_leakage {
        if let Some(val) = pl.expired_open_value_eur {
            let per_year = if pl.window_years > 0.0 { val / pl.window_years } else { val };
            findings.push(Finding {
                key: "plan_leakage".into(),
                title: "Genehmigte Behandlungspläne verfallen".into(),
                verdict: if per_year > 10_000.0 { Verdict::Chance } else { Verdict::Solide },
                summary: format!(
                    "{} genehmigte Pläne über der Gültigkeitsfrist, nie eingegliedert — {} Behandlungswert.",
                    pl.expired_open_cases, eur(val)
                ),
                evidence: vec![
                    format!("Bestand: {} Fälle = {} ({} je Fall)", pl.expired_open_cases, eur(val),
                        eur(if pl.expired_open_cases > 0 { val / pl.expired_open_cases as f64 } else { 0.0 })),
                    format!("≈ {} neu verfallender Behandlungswert pro Jahr", eur(per_year)),
                    format!("Dazu {} bewusst deaktivierte Fälle{} (Patientenentscheidung, kein Hebel)",
                        pl.deactivated_cases,
                        pl.deactivated_value_eur.map(|v| format!(" = {}", eur(v))).unwrap_or_default()),
                ],
                potential_eur_min: Some(per_year * PLAN_RESCUE_MIN),
                potential_eur_max: Some(per_year * PLAN_RESCUE_MAX),
                recommendation: "Arbeitsliste „genehmigt & nicht terminiert" mit Frist-Countdown; Patienten vor Ablauf aktiv terminieren (Praxishub-HKP-Tracking).".into(),
            });
        }
    }

    // 5) PAR-/UPT-Strecken.
    if let Some(par) = &inputs.par {
        let potential = par.never_started as f64 * PAR_START_VALUE
            + par.upt_broken as f64 * UPT_VALUE_PER_YEAR;
        findings.push(Finding {
            key: "par_upt".into(),
            title: "PAR-Strecken & UPT-Compliance".into(),
            verdict: if potential > 5_000.0 { Verdict::Chance } else { Verdict::Solide },
            summary: format!(
                "{:.0} % der genehmigten PAR-Pläne nie begonnen; {:.0} % der UPT-Strecken abgerissen.",
                pct(par.never_started, par.approved_plans),
                pct(par.upt_broken, par.upt_expected)
            ),
            evidence: vec![
                format!("{} von {} genehmigten PAR-Plänen ohne Behandlungsbeginn", par.never_started, par.approved_plans),
                format!("{} von {} UPT-Strecken ohne Termin im Soll-Intervall", par.upt_broken, par.upt_expected),
                format!("Ansatz: {} je nicht begonnener Strecke, {}/Jahr je UPT-Abriss (extrabudgetär)",
                    eur(PAR_START_VALUE), eur(UPT_VALUE_PER_YEAR)),
            ],
            potential_eur_min: Some(potential * 0.6),
            potential_eur_max: Some(potential),
            recommendation: "UPT-Recall mit befundgerechten Intervallen; nicht begonnene Strecken nachterminieren.".into(),
        });
    }

    // 6) Kleinbetrags-Factoring.
    if let Some(f) = &inputs.factoring_small {
        findings.push(Finding {
            key: "factoring_kleinbetrag".into(),
            title: "Factoring-Gebühren für Kleinbeträge".into(),
            verdict: if f.volume_12m_eur * FACTORING_SAVE_MIN > 3_000.0 { Verdict::Chance } else { Verdict::Info },
            summary: format!(
                "{} Rechnungen unter {} laufen übers Rechenzentrum ({} Volumen/Jahr).",
                f.invoices_12m, eur(f.threshold_eur), eur(f.volume_12m_eur)
            ),
            evidence: vec![
                format!("Netto-Ersparnis bei Direkteinzug: {:.0}–{:.0} % des Volumens",
                    FACTORING_SAVE_MIN * 100.0, FACTORING_SAVE_MAX * 100.0),
                "Vorher prüfen: Andienungspflicht/Volumenstaffel im RZ-Vertrag".into(),
            ],
            potential_eur_min: Some(f.volume_12m_eur * FACTORING_SAVE_MIN),
            potential_eur_max: Some(f.volume_12m_eur * FACTORING_SAVE_MAX),
            recommendation: "Kleinbeträge direkt beim Termin kassieren (Karte/Zahlungslink) statt fakturieren.".into(),
        });
    }

    // 7) Forderungslage — bewusst auch als GUTES Ergebnis berichten.
    if let Some(r) = &inputs.receivables {
        let rev = inputs.revenue_last_year_eur.unwrap_or(0.0);
        let quote = if rev > 0.0 { r.written_off_avg_year_eur / rev * 100.0 } else { 0.0 };
        let solide = quote < 0.5 && r.open_direct_eur < 10_000.0;
        findings.push(Finding {
            key: "forderungen".into(),
            title: "Zahlungsausfälle & offene Forderungen".into(),
            verdict: if solide { Verdict::Solide } else { Verdict::Chance },
            summary: if solide {
                "Kein Handlungsbedarf: Ausfälle und Offenstände sind minimal — das Forderungsmanagement funktioniert.".into()
            } else {
                "Auffällige Ausfälle/Offenstände — Forderungsmanagement prüfen.".into()
            },
            evidence: vec![
                format!("Ausgebuchte Forderungen: Ø {}/Jahr ({quote:.2} % vom Umsatz)", eur(r.written_off_avg_year_eur)),
                format!("Offene Direktforderungen aktuell: {}", eur(r.open_direct_eur)),
            ],
            potential_eur_min: if solide { None } else { Some(r.written_off_avg_year_eur * 0.3) },
            potential_eur_max: if solide { None } else { Some(r.written_off_avg_year_eur * 0.6) },
            recommendation: if solide {
                "So lassen — Aufwand hier bringt nichts.".into()
            } else {
                "Systematische Zahlungserinnerungen + Inkasso-Anbindung.".into()
            },
        });
    }

    // 8) Abrechnungsdisziplin — auch das Positive berichten.
    if let Some(u) = inputs.unbilled_private_eur {
        let solide = u < 5_000.0;
        findings.push(Finding {
            key: "abrechnung".into(),
            title: "Erbrachte, nicht abgerechnete Leistungen".into(),
            verdict: if solide { Verdict::Solide } else { Verdict::Chance },
            summary: if solide {
                "Die Abrechnung läuft sauber: praktisch keine erbrachten Privatleistungen ohne Rechnung.".into()
            } else {
                format!("{} erbrachte Privatleistungen warten auf Abrechnung.", eur(u))
            },
            evidence: vec![format!("Unfakturierter Bestand: {}", eur(u))],
            potential_eur_min: if solide { None } else { Some(u * 0.8) },
            potential_eur_max: if solide { None } else { Some(u) },
            recommendation: if solide { "Kein Handlungsbedarf.".into() } else { "Abrechnungs-Rückstand abarbeiten (Einmaleffekt).".into() },
        });
    }

    // 9) No-Shows — meist nur als Datenlücke benennbar.
    if let Some(n) = inputs.no_shows_documented_12m {
        findings.push(Finding {
            key: "no_shows".into(),
            title: "Terminausfälle (No-Shows)".into(),
            verdict: Verdict::Info,
            summary: format!("Im PVS sind {n} Terminausfälle in 12 Monaten dokumentiert — die echte Zahl steht im Kalendersystem und liegt erfahrungsgemäß deutlich höher."),
            evidence: vec!["PVS-Kartei erfasst No-Shows nur unsystematisch (Untergrenze).".into()],
            potential_eur_min: None,
            potential_eur_max: None,
            recommendation: "Terminerinnerungen + Ausfallhonorar-Prozess (Vereinbarung, Rechnung, Erinnerung, Inkasso) über die Kalenderdaten.".into(),
        });
    }

    let (mut min, mut max) = (0.0, 0.0);
    for f in &findings {
        min += f.potential_eur_min.unwrap_or(0.0);
        max += f.potential_eur_max.unwrap_or(0.0);
    }
    PotentialReport {
        pvs: inputs.pvs.clone(),
        stichtag: inputs.stichtag.clone(),
        inputs: inputs.clone(),
        findings,
        total_potential_eur_min: min,
        total_potential_eur_max: max,
    }
}

// ── HTML-Report (Sales / lokal, selbst-enthalten) ────────────────────────────

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Rendert den Report als eigenständige deutsche HTML-Seite (keine externen
/// Ressourcen — direkt versend-/druckbar für den Sales-Prozess).
pub fn render_html(r: &PotentialReport) -> String {
    let date = format!(
        "{}.{}.{}",
        &r.stichtag.get(6..8).unwrap_or("??"),
        &r.stichtag.get(4..6).unwrap_or("??"),
        &r.stichtag.get(0..4).unwrap_or("????")
    );
    let mut s = String::new();
    s.push_str("<!DOCTYPE html><html lang=\"de\"><head><meta charset=\"utf-8\">");
    s.push_str("<title>Praxishub Potenzialanalyse</title><style>");
    s.push_str(
        "body{font-family:system-ui,Segoe UI,sans-serif;max-width:860px;margin:32px auto;padding:0 16px;color:#1a1a1a;line-height:1.45}\
         h1{font-size:26px;margin-bottom:4px} .sub{color:#666;margin-bottom:24px}\
         .total{background:#f0f7f2;border:1px solid #bcd9c4;border-radius:10px;padding:16px 20px;font-size:19px;margin-bottom:28px}\
         .card{border:1px solid #ddd;border-radius:10px;padding:16px 20px;margin-bottom:14px}\
         .chance{border-left:6px solid #d97706} .solide{border-left:6px solid #16a34a} .info{border-left:6px solid #64748b}\
         .badge{display:inline-block;font-size:12px;font-weight:600;border-radius:99px;padding:2px 10px;margin-left:8px;vertical-align:middle}\
         .chance .badge{background:#fef3c7;color:#92400e} .solide .badge{background:#dcfce7;color:#166534} .info .badge{background:#e2e8f0;color:#334155}\
         .pot{float:right;font-weight:700;font-size:17px} ul{margin:8px 0 8px 18px;padding:0;color:#444}\
         .rec{background:#f8fafc;border-radius:6px;padding:8px 12px;margin-top:8px;font-size:14px}\
         .foot{color:#888;font-size:12px;margin-top:28px}",
    );
    s.push_str("</style></head><body>");
    s.push_str("<h1>Potenzialanalyse</h1>");
    s.push_str(&format!(
        "<div class=\"sub\">Datenbasis: {} · Stichtag {} · automatisch erhoben durch den Praxishub Connector (nur Lesezugriff)</div>",
        esc(&r.pvs), date
    ));
    s.push_str(&format!(
        "<div class=\"total\"><b>Identifiziertes Jahres-Potenzial: {} – {}</b><br><span style=\"font-size:14px;color:#446\">konservativ gerechnet; Annahmen stehen bei jedem Befund</span></div>",
        eur(r.total_potential_eur_min), eur(r.total_potential_eur_max)
    ));
    for f in &r.findings {
        let (class, label) = match f.verdict {
            Verdict::Chance => ("chance", "Chance"),
            Verdict::Solide => ("solide", "Läuft gut"),
            Verdict::Info => ("info", "Hinweis"),
        };
        s.push_str(&format!("<div class=\"card {class}\">"));
        if let (Some(min), Some(max)) = (f.potential_eur_min, f.potential_eur_max) {
            s.push_str(&format!("<span class=\"pot\">{} – {}/Jahr</span>", eur(min), eur(max)));
        }
        s.push_str(&format!(
            "<b>{}</b><span class=\"badge\">{label}</span><div style=\"margin-top:6px\">{}</div>",
            esc(&f.title),
            esc(&f.summary)
        ));
        s.push_str("<ul>");
        for e in &f.evidence {
            s.push_str(&format!("<li>{}</li>", esc(e)));
        }
        s.push_str("</ul>");
        s.push_str(&format!("<div class=\"rec\"><b>Maßnahme:</b> {}</div>", esc(&f.recommendation)));
        s.push_str("</div>");
    }
    s.push_str("<div class=\"foot\">Alle Zahlen stammen read-only aus dem Praxisverwaltungssystem der Praxis. Potenziale sind konservative Schätzungen (Umsatz, nicht Ertrag); Kapazitätsgrenzen der Praxis sind zu berücksichtigen.</div>");
    s.push_str("</body></html>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zmm_inputs() -> AnalysisInputs {
        // Kalibrierdaten des Z1-Piloten (2026-07-13).
        AnalysisInputs {
            pvs: "Z1".into(),
            stichtag: "20260713".into(),
            active_patients_12m: Some(3699),
            revenue_last_year_eur: Some(2_596_162.0),
            returning_base: Some(3507),
            returning_seen_again: Some(2600),
            inactive_12_24m: Some(1219),
            inactive_24_36m: Some(710),
            inactive_over_36m: Some(388),
            prophylaxe: Some(ProphylaxeStats {
                active_patients: 3699,
                without_pzr: 1874,
                freq_1x: 1439,
                freq_2x: 351,
                freq_3plus: 35,
                avg_price_eur: 123.43,
                services_12m: 2254,
            }),
            plan_leakage: Some(PlanLeakage {
                expired_open_cases: 146,
                expired_open_value_eur: Some(255_661.0),
                deactivated_cases: 65,
                deactivated_value_eur: Some(107_454.0),
                window_years: 2.0,
            }),
            par: Some(ParStats { approved_plans: 217, never_started: 43, upt_expected: 156, upt_broken: 43 }),
            receivables: Some(Receivables { written_off_avg_year_eur: 2_900.0, open_direct_eur: 1_258.0 }),
            unbilled_private_eur: Some(0.0),
            factoring_small: Some(FactoringSmall { invoices_12m: 3000, volume_12m_eur: 370_000.0, threshold_eur: 200.0 }),
            no_shows_documented_12m: Some(15),
        }
    }

    #[test]
    fn zmm_kalibrierung_liefert_erwartete_groessenordnung() {
        let r = evaluate(&zmm_inputs());
        // Prophylaxe: 1439×0,4×123,43 + 1874×0,2×123,43 ≈ 117 k€ (max).
        let p = r.findings.iter().find(|f| f.key == "prophylaxe").unwrap();
        assert_eq!(p.verdict, Verdict::Chance);
        let max = p.potential_eur_max.unwrap();
        assert!((110_000.0..125_000.0).contains(&max), "{max}");
        // Plan-Leakage: 255k/2 Jahre × 30–40 % ≈ 38–51 k€.
        let pl = r.findings.iter().find(|f| f.key == "plan_leakage").unwrap();
        assert!((35_000.0..42_000.0).contains(&pl.potential_eur_min.unwrap()));
        // Forderungen + Abrechnung: bewusst als SOLIDE berichtet.
        assert_eq!(r.findings.iter().find(|f| f.key == "forderungen").unwrap().verdict, Verdict::Solide);
        assert_eq!(r.findings.iter().find(|f| f.key == "abrechnung").unwrap().verdict, Verdict::Solide);
        // Gesamtsumme plausibel (nur Chancen zählen).
        assert!(r.total_potential_eur_max > 150_000.0 && r.total_potential_eur_max < 400_000.0);
    }

    #[test]
    fn teilbefuellte_inputs_erzeugen_teilreport() {
        // Ein PVS-Adapter, der nur Prophylaxe liefert (z. B. erste charly-Version).
        let inputs = AnalysisInputs {
            pvs: "charly".into(),
            stichtag: "20260713".into(),
            prophylaxe: Some(ProphylaxeStats {
                active_patients: 1000,
                without_pzr: 500,
                freq_1x: 300,
                freq_2x: 180,
                freq_3plus: 20,
                avg_price_eur: 110.0,
                services_12m: 700,
            }),
            ..Default::default()
        };
        let r = evaluate(&inputs);
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].key, "prophylaxe");
        assert!(r.total_potential_eur_max > 0.0);
    }

    #[test]
    fn html_rendert_alle_befunde_und_summe() {
        let r = evaluate(&zmm_inputs());
        let html = render_html(&r);
        assert!(html.contains("Potenzialanalyse"));
        assert!(html.contains("Prophylaxe"));
        assert!(html.contains("Läuft gut")); // Positiv-Befunde sichtbar
        assert!(html.contains("Identifiziertes Jahres-Potenzial"));
        assert!(!html.contains("<script")); // selbst-enthalten, kein JS nötig
    }

    #[test]
    fn euro_format_deutsch() {
        assert_eq!(eur(255661.32), "255.661 €");
        assert_eq!(eur(0.4), "0 €");
        assert_eq!(eur(1234567.0), "1.234.567 €");
    }
}
