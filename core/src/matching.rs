//! Robuste Patienten-Zuordnung für die Dokumentenablage.
//!
//! Der Patient tippt Name und Geburtsdatum selbst ins Anamnese-Formular; die
//! Z1-/PraxisArchiv-Stammdaten sind fremdgepflegt. Ein naiver String-Vergleich
//! scheitert daher an Formatunterschieden (`20010223` vs. `23.02.2001`),
//! Groß-/Kleinschreibung, Umlaut-Schreibweisen (`Müller`/`Mueller`) und
//! Leerzeichen. Dieses Modul normalisiert beide Seiten auf eine Vergleichsform.
//!
//! **Sicherheitsprinzip:** Lieber gar nicht zuordnen als falsch zuordnen. Der
//! Primärschlüssel ist **Nachname + Vorname + Geburtsdatum** (Vorname ist Pflicht,
//! sonst würden Zwillinge — gleicher Nachname, gleiches Geburtsdatum — vertauscht).
//! Bei mehreren gleichwertigen Treffern entscheiden optionale Zusatzfelder (PLZ,
//! E-Mail); bleibt es mehrdeutig, wird nicht abgelegt.

/// Kanonischer Vergleichsschlüssel eines Patienten. Alle Felder sind bereits
/// normalisiert (siehe [`normalize_name`] / [`normalize_birthdate`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatientKey {
    pub last_name: String,
    pub first_name: String,
    /// Geburtsdatum kanonisch als `JJJJMMTT`, oder leer wenn unparsbar.
    pub birth_date: String,
}

impl PatientKey {
    /// Baut den Schlüssel aus Rohwerten (beliebiges Datumsformat, beliebige
    /// Groß-/Kleinschreibung, Umlaute). `birth_date` akzeptiert `TT.MM.JJJJ`,
    /// `JJJJMMTT`, `JJJJ-MM-TT` sowie mit `/` statt `.`/`-`.
    pub fn new(last_name: &str, first_name: &str, birth_date: &str) -> Self {
        Self {
            last_name: normalize_name(last_name),
            first_name: normalize_name(first_name),
            birth_date: normalize_birthdate(birth_date).unwrap_or_default(),
        }
    }

    /// Genug Substanz für einen belastbaren Match? Ohne Nachname **und**
    /// Geburtsdatum ordnen wir grundsätzlich nichts zu.
    pub fn is_usable(&self) -> bool {
        !self.last_name.is_empty() && !self.birth_date.is_empty()
    }

    /// Starker Match: Nachname + Vorname + Geburtsdatum stimmen (nach
    /// Normalisierung) überein. Vorname MUSS auf beiden Seiten vorhanden sein —
    /// fehlt er, ist das kein starker Match (schützt vor Zwillings-Vertauschung).
    pub fn matches(&self, other: &PatientKey) -> bool {
        if !self.is_usable() || !other.is_usable() {
            return false;
        }
        if self.last_name != other.last_name || self.birth_date != other.birth_date {
            return false;
        }
        !self.first_name.is_empty()
            && !other.first_name.is_empty()
            && self.first_name == other.first_name
    }
}

/// Normalisiert einen Namen auf eine vergleichbare Form: Kleinbuchstaben,
/// deutsche Umlaute/ß entfaltet (ä→ae, ö→oe, ü→ue, ß→ss), Akzente entfernt und
/// alles außer `a–z`/`0–9` verworfen (Bindestrich, Leerzeichen, Punkt, Apostroph
/// fallen weg). So matchen `Müller`≡`Mueller`, `von der Berg`≡`vonderberg`,
/// `O'Neil`≡`oneil`.
pub fn normalize_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.trim().chars() {
        match ch {
            'ä' | 'Ä' => out.push_str("ae"),
            'ö' | 'Ö' => out.push_str("oe"),
            'ü' | 'Ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            'à' | 'á' | 'â' | 'ã' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Å' => out.push('a'),
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => out.push('i'),
            'ò' | 'ó' | 'ô' | 'õ' | 'Ò' | 'Ó' | 'Ô' | 'Õ' => out.push('o'),
            'ù' | 'ú' | 'û' | 'Ù' | 'Ú' | 'Û' => out.push('u'),
            'ç' | 'Ç' => out.push('c'),
            'ñ' | 'Ñ' => out.push('n'),
            c if c.is_ascii_alphanumeric() => out.push(c.to_ascii_lowercase()),
            _ => {} // Leerzeichen, Bindestrich, Punkt, Apostroph etc. verwerfen
        }
    }
    out
}

/// Bringt ein Geburtsdatum auf die kanonische Form `JJJJMMTT`. Akzeptiert:
/// `TT.MM.JJJJ`, `JJJJMMTT` (Z1/PraxisArchiv), `JJJJ-MM-TT` (ISO) sowie `/` oder
/// `-` als Trenner im TT.MM.JJJJ-Format. Gibt `None` zurück, wenn kein plausibles
/// Datum erkennbar ist — dann findet KEIN Datums-Match statt (sicherer Default).
pub fn normalize_birthdate(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }

    // Reine Ziffernfolge? Dann JJJJMMTT (8 Stellen, mit Jahr voran).
    if s.chars().all(|c| c.is_ascii_digit()) {
        if s.len() == 8 {
            return validate_ymd(&s[0..4], &s[4..6], &s[6..8]);
        }
        return None;
    }

    // Sonst über Trenner zerlegen (., -, /).
    let parts: Vec<&str> = s.split(['.', '-', '/']).filter(|p| !p.is_empty()).collect();
    if parts.len() != 3 {
        return None;
    }

    // ISO (JJJJ-MM-TT): erstes Feld vierstellig. Sonst deutsches TT.MM.JJJJ.
    if parts[0].len() == 4 {
        validate_ymd(parts[0], parts[1], parts[2])
    } else {
        validate_ymd(parts[2], parts[1], parts[0])
    }
}

/// Prüft Jahr/Monat/Tag grob auf Plausibilität und formatiert als `JJJJMMTT`.
fn validate_ymd(y: &str, m: &str, d: &str) -> Option<String> {
    let year: u32 = y.parse().ok()?;
    let month: u32 = m.parse().ok()?;
    let day: u32 = d.parse().ok()?;
    if !(1900..=2200).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!("{year:04}{month:02}{day:02}"))
}

/// Wählt aus mehreren Stammdaten-Kandidaten (z. B. Ergebnis eines Namens-Lookups
/// in der PraxisArchiv-DB) den eindeutigen Treffer für den gesuchten Patienten.
///
/// Ablauf:
///   1. Auf starke Matches ([`PatientKey::matches`]) filtern.
///   2. Genau einer → das ist er.
///   3. Mehrere → optionale Tiebreaker anwenden (PLZ, E-Mail — normalisiert). Nur
///      wenn danach **genau einer** übrig bleibt, gilt er als eindeutig.
///   4. Keiner / weiter mehrdeutig → [`MatchResult::None`] bzw. `Ambiguous`.
///
/// `candidates` liefert je Kandidat den Schlüssel und optionale Zusatzfelder.
pub fn resolve_unique<'a, T>(
    wanted: &PatientKey,
    wanted_zip: Option<&str>,
    wanted_email: Option<&str>,
    candidates: &'a [Candidate<T>],
) -> MatchResult<&'a T> {
    let strong: Vec<&Candidate<T>> =
        candidates.iter().filter(|c| wanted.matches(&c.key)).collect();

    match strong.as_slice() {
        [] => MatchResult::None,
        [only] => MatchResult::Unique(&only.payload),
        many => {
            // Tiebreaker: PLZ, dann E-Mail. Jeweils nur anwenden, wenn der
            // gesuchte Wert vorhanden ist und die Menge echt verkleinert.
            let narrowed = narrow_by(many, wanted_zip, |c| c.zip.as_deref());
            let narrowed = narrow_by(&narrowed, wanted_email, |c| c.email.as_deref());
            match narrowed.as_slice() {
                [only] => MatchResult::Unique(&only.payload),
                _ => MatchResult::Ambiguous(narrowed.len().max(many.len())),
            }
        }
    }
}

/// Verkleinert die Kandidatenmenge über ein normalisiertes Zusatzfeld, sofern der
/// gesuchte Wert bekannt ist und mindestens ein Kandidat exakt passt. Passt keiner
/// oder ist der Wert unbekannt, bleibt die Menge unverändert (das Feld darf einen
/// echten Match nie wegfiltern, nur zwischen Gleichstand-Kandidaten entscheiden).
fn narrow_by<'a, T>(
    set: &[&'a Candidate<T>],
    wanted: Option<&str>,
    field: impl Fn(&Candidate<T>) -> Option<&str>,
) -> Vec<&'a Candidate<T>> {
    let Some(w) = wanted.map(normalize_name).filter(|s| !s.is_empty()) else {
        return set.to_vec();
    };
    let hits: Vec<&Candidate<T>> = set
        .iter()
        .copied()
        .filter(|c| field(c).map(normalize_name).is_some_and(|v| v == w))
        .collect();
    if hits.is_empty() {
        set.to_vec()
    } else {
        hits
    }
}

/// Ein Stammdaten-Kandidat aus dem Lookup: Vergleichsschlüssel, optionale
/// Tiebreaker-Felder und die zurückzugebende Nutzlast (z. B. die PatientenID).
#[derive(Debug, Clone)]
pub struct Candidate<T> {
    pub key: PatientKey,
    pub zip: Option<String>,
    pub email: Option<String>,
    pub payload: T,
}

/// Ergebnis einer eindeutigen Zuordnung.
#[derive(Debug, PartialEq, Eq)]
pub enum MatchResult<T> {
    /// Genau ein eindeutiger Treffer.
    Unique(T),
    /// Kein Treffer (Patient (noch) nicht in den Stammdaten) — später erneut versuchen.
    None,
    /// Mehrere gleichwertige Treffer, auch nach Tiebreakern — bewusst NICHT ablegen.
    /// Der Wert ist die Zahl verbleibender Kandidaten (für Logging/Sichtbarkeit).
    Ambiguous(usize),
}

// ── Fuzzy-Zuordnung (höhere Trefferquote) ────────────────────────────────────
//
// Ziel: möglichst viele Aufnahmen automatisch zuordnen (leichte Tippfehler in
// Namen/Geburtsdatum tolerieren, PLZ als starken Bestätiger nutzen), ABER bei
// Unsicherheit NIE raten — dann geht der Fall zur manuellen Zuordnung ans Team.

/// Edit-Distanz (Damerau/OSA): Einfügen/Löschen/Ersetzen **und benachbarte
/// Transposition** je 1 Edit — so zählt „Groth"↔„Groht" als 1 (häufiger Tippfehler).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..=n {
        d[i][0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1); // benachbarte Transposition
            }
            d[i][j] = v;
        }
    }
    d[n][m]
}

/// Konfidenz-Score (0–115) eines Kandidaten gegen den Gesuchten. Namen/Geburts-
/// datum sind bereits normalisiert (siehe [`PatientKey`]); leichte Tippfehler
/// zählen abgestuft, PLZ-Gleichheit gibt einen Bonus.
pub fn confidence(w: &PatientKey, w_zip: Option<&str>, c: &PatientKey, c_zip: Option<&str>) -> u32 {
    let ld = edit_distance(&w.last_name, &c.last_name);
    let fd = edit_distance(&w.first_name, &c.first_name);
    let dob_eq = !w.birth_date.is_empty() && w.birth_date == c.birth_date;
    let dob_close = !dob_eq
        && !w.birth_date.is_empty()
        && !c.birth_date.is_empty()
        && edit_distance(&w.birth_date, &c.birth_date) <= 1;
    let plz_eq = matches!(
        (
            w_zip.map(normalize_name).filter(|s| !s.is_empty()),
            c_zip.map(normalize_name).filter(|s| !s.is_empty()),
        ),
        (Some(a), Some(b)) if a == b
    );

    let mut s = 0u32;
    s += match ld {
        0 => 45,
        1 => 33,
        2 => 20,
        _ => 0,
    };
    if !w.first_name.is_empty() && !c.first_name.is_empty() {
        s += match fd {
            0 => 25,
            1 => 17,
            2 => 8,
            _ => 0,
        };
    }
    if dob_eq {
        s += 30;
    } else if dob_close {
        s += 12;
    }
    if plz_eq {
        s += 15;
    }
    s
}

/// Auto-Match ab diesem Score (und mit klarem Vorsprung), sonst Review.
pub const ACCEPT: u32 = 85;
/// Ab diesem Score gilt ein Kandidat als „nah dran" → Review statt NotFound.
pub const REVIEW: u32 = 55;
const MIN_LEAD: u32 = 10;

/// Ergebnis der Fuzzy-Zuordnung.
#[derive(Debug, PartialEq, Eq)]
pub enum Resolution<T> {
    /// Sicher genug → automatisch zuordnen.
    Matched(T),
    /// Nah dran, aber unsicher/mehrdeutig → **manuell** ans Team (mit Kandidaten).
    Review(Vec<T>),
    /// Niemand nah genug → Patient (noch) nicht in Z1 → später erneut versuchen.
    NotFound,
}

/// Ordnet den Gesuchten einem Kandidaten zu: bester Score ≥ [`ACCEPT`] **und** mit
/// klarem Vorsprung → [`Resolution::Matched`]; mind. ein Kandidat ≥ [`REVIEW`], aber
/// unsicher → [`Resolution::Review`]; sonst [`Resolution::NotFound`].
pub fn resolve_fuzzy<T: Clone>(
    wanted: &PatientKey,
    wanted_zip: Option<&str>,
    candidates: &[Candidate<T>],
) -> Resolution<T> {
    if wanted.last_name.is_empty() {
        return Resolution::NotFound;
    }
    let mut scored: Vec<(u32, &Candidate<T>)> = candidates
        .iter()
        .map(|c| (confidence(wanted, wanted_zip, &c.key, c.zip.as_deref()), c))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    let Some(&(best, top)) = scored.first() else {
        return Resolution::NotFound;
    };
    if best < REVIEW {
        return Resolution::NotFound;
    }
    let second = scored.get(1).map(|x| x.0).unwrap_or(0);
    if best >= ACCEPT && best - second >= MIN_LEAD {
        return Resolution::Matched(top.payload.clone());
    }
    let near: Vec<T> = scored
        .iter()
        .filter(|(s, _)| *s >= REVIEW)
        .map(|(_, c)| c.payload.clone())
        .collect();
    Resolution::Review(near)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisiert_umlaute_und_schreibweise() {
        assert_eq!(normalize_name("Müller"), "mueller");
        assert_eq!(normalize_name("Mueller"), "mueller");
        assert_eq!(normalize_name("  Groß-Schön  "), "grossschoen");
        assert_eq!(normalize_name("O'Neil"), "oneil");
        assert_eq!(normalize_name("von der Berg"), "vonderberg");
        assert_eq!(normalize_name("José"), "jose");
    }

    #[test]
    fn normalisiert_geburtsdatum_formate() {
        // Backend liefert TT.MM.JJJJ, Z1/PA liefert JJJJMMTT — beide → gleiche Form.
        assert_eq!(normalize_birthdate("23.02.2001").as_deref(), Some("20010223"));
        assert_eq!(normalize_birthdate("20010223").as_deref(), Some("20010223"));
        assert_eq!(normalize_birthdate("2001-02-23").as_deref(), Some("20010223"));
        assert_eq!(normalize_birthdate("23/02/2001").as_deref(), Some("20010223"));
        assert_eq!(normalize_birthdate("1.1.1980").as_deref(), Some("19800101"));
    }

    #[test]
    fn verwirft_unplausible_daten() {
        assert_eq!(normalize_birthdate(""), None);
        assert_eq!(normalize_birthdate("keinDatum"), None);
        assert_eq!(normalize_birthdate("32.13.2001"), None);
        assert_eq!(normalize_birthdate("2001"), None);
        assert_eq!(normalize_birthdate("230220"), None); // 6-stellig, nicht unterstützt
    }

    #[test]
    fn match_ueber_format_und_umlautgrenze() {
        let formular = PatientKey::new("Müller", "Sören", "23.02.2001");
        let z1 = PatientKey::new("Mueller", "Soeren", "20010223");
        assert!(formular.matches(&z1));
    }

    #[test]
    fn zwillinge_werden_nicht_vertauscht() {
        // Gleicher Nachname, gleiches Geburtsdatum, unterschiedlicher Vorname.
        let lisa = PatientKey::new("Groth", "Lisa", "23.02.2001");
        let lena = PatientKey::new("Groth", "Lena", "23.02.2001");
        assert!(!lisa.matches(&lena));
    }

    #[test]
    fn kein_match_ohne_vorname() {
        // Fehlt der Vorname auf einer Seite, ist es kein starker Match.
        let ohne = PatientKey::new("Groth", "", "23.02.2001");
        let mit = PatientKey::new("Groth", "Nikolas", "23.02.2001");
        assert!(!ohne.matches(&mit));
    }

    #[test]
    fn kein_match_ohne_geburtsdatum() {
        let a = PatientKey::new("Groth", "Nikolas", "");
        let b = PatientKey::new("Groth", "Nikolas", "23.02.2001");
        assert!(!a.matches(&b));
        assert!(!a.is_usable());
    }

    fn cand(last: &str, first: &str, dob: &str, zip: Option<&str>, email: Option<&str>, id: &str) -> Candidate<String> {
        Candidate {
            key: PatientKey::new(last, first, dob),
            zip: zip.map(str::to_string),
            email: email.map(str::to_string),
            payload: id.to_string(),
        }
    }

    #[test]
    fn resolve_eindeutiger_treffer() {
        let wanted = PatientKey::new("Groth", "Nikolas", "23.02.2001");
        let cands = vec![
            cand("Groth", "Nikolas", "23.02.2001", None, None, "16006"),
            cand("Meier", "Anna", "01.01.1990", None, None, "17000"),
        ];
        assert_eq!(resolve_unique(&wanted, None, None, &cands), MatchResult::Unique(&"16006".to_string()));
    }

    #[test]
    fn resolve_kein_treffer() {
        let wanted = PatientKey::new("Groth", "Nikolas", "23.02.2001");
        let cands = vec![cand("Meier", "Anna", "01.01.1990", None, None, "17000")];
        assert_eq!(resolve_unique(&wanted, None, None, &cands), MatchResult::None);
    }

    #[test]
    fn resolve_zwillinge_ohne_tiebreaker_mehrdeutig() {
        // Zwei echte Namensvettern (gleicher Vorname+Nachname+GebDat) ohne PLZ/Email.
        let wanted = PatientKey::new("Groth", "Max", "23.02.2001");
        let cands = vec![
            cand("Groth", "Max", "23.02.2001", None, None, "16006"),
            cand("Groth", "Max", "23.02.2001", None, None, "18001"),
        ];
        assert_eq!(resolve_unique(&wanted, None, None, &cands), MatchResult::Ambiguous(2));
    }

    #[test]
    fn resolve_tiebreaker_plz_entscheidet() {
        let wanted = PatientKey::new("Groth", "Max", "23.02.2001");
        let cands = vec![
            cand("Groth", "Max", "23.02.2001", Some("10709"), None, "16006"),
            cand("Groth", "Max", "23.02.2001", Some("80331"), None, "18001"),
        ];
        assert_eq!(
            resolve_unique(&wanted, Some("10709"), None, &cands),
            MatchResult::Unique(&"16006".to_string())
        );
    }

    #[test]
    fn resolve_tiebreaker_email_entscheidet() {
        let wanted = PatientKey::new("Groth", "Max", "23.02.2001");
        let cands = vec![
            cand("Groth", "Max", "23.02.2001", None, Some("max@a.de"), "16006"),
            cand("Groth", "Max", "23.02.2001", None, Some("max@b.de"), "18001"),
        ];
        assert_eq!(
            resolve_unique(&wanted, None, Some("max@a.de"), &cands),
            MatchResult::Unique(&"16006".to_string())
        );
    }

    #[test]
    fn tiebreaker_filtert_echten_match_nicht_weg() {
        // Gesuchte PLZ passt zu keinem Kandidaten → PLZ darf nicht alle wegfiltern;
        // bleibt mehrdeutig statt fälschlich leer.
        let wanted = PatientKey::new("Groth", "Max", "23.02.2001");
        let cands = vec![
            cand("Groth", "Max", "23.02.2001", Some("99999"), None, "16006"),
            cand("Groth", "Max", "23.02.2001", Some("88888"), None, "18001"),
        ];
        assert_eq!(resolve_unique(&wanted, Some("10709"), None, &cands), MatchResult::Ambiguous(2));
    }

    // ── Fuzzy ────────────────────────────────────────────────────────────────
    #[test]
    fn edit_distance_basis() {
        assert_eq!(edit_distance("groth", "groth"), 0);
        assert_eq!(edit_distance("groth", "groht"), 1); // benachbarte Transposition = 1
        assert_eq!(edit_distance("meier", "mayer"), 2); // zwei Ersetzungen (keine Transposition)
        assert_eq!(edit_distance("", "abc"), 3);
    }

    #[test]
    fn fuzzy_exakt_wird_gematcht() {
        let w = PatientKey::new("Groth", "Nikolas", "23.02.2001");
        let c = vec![cand("Groth", "Nikolas", "23.02.2001", Some("10709"), None, "16006")];
        assert_eq!(resolve_fuzzy(&w, Some("10709"), &c), Resolution::Matched("16006".to_string()));
    }

    #[test]
    fn fuzzy_namenstippfehler_mit_gebdat_wird_gematcht() {
        // "Groht" statt "Groth" (1 Edit), Vorname + Geburtsdatum exakt → Auto-Match.
        let w = PatientKey::new("Groht", "Nikolas", "23.02.2001");
        let c = vec![cand("Groth", "Nikolas", "23.02.2001", None, None, "16006")];
        assert_eq!(resolve_fuzzy(&w, None, &c), Resolution::Matched("16006".to_string()));
    }

    #[test]
    fn fuzzy_gebdat_tippfehler_mit_plz_wird_gematcht() {
        // Geburtsdatum um eine Ziffer daneben, aber Name exakt + PLZ passt → Match.
        let w = PatientKey::new("Groth", "Nikolas", "23.02.2001");
        let c = vec![cand("Groth", "Nikolas", "23.02.2011", Some("10709"), None, "16006")];
        assert_eq!(resolve_fuzzy(&w, Some("10709"), &c), Resolution::Matched("16006".to_string()));
    }

    #[test]
    fn fuzzy_zwillinge_gehen_ins_review() {
        // Zwei identische Namensvettern ohne unterscheidende PLZ → Review, nicht raten.
        let w = PatientKey::new("Groth", "Max", "23.02.2001");
        let c = vec![
            cand("Groth", "Max", "23.02.2001", None, None, "16006"),
            cand("Groth", "Max", "23.02.2001", None, None, "18001"),
        ];
        match resolve_fuzzy(&w, None, &c) {
            Resolution::Review(v) => assert_eq!(v.len(), 2),
            other => panic!("erwartet Review, war {other:?}"),
        }
    }

    #[test]
    fn fuzzy_gebdat_daneben_ohne_plz_geht_ins_review() {
        // Geburtsdatum daneben, keine PLZ-Bestätigung → nicht sicher genug → Review.
        let w = PatientKey::new("Groth", "Nikolas", "23.02.2001");
        let c = vec![cand("Groth", "Nikolas", "23.02.2011", None, None, "16006")];
        assert!(matches!(resolve_fuzzy(&w, None, &c), Resolution::Review(_)));
    }

    #[test]
    fn fuzzy_niemand_nah_ist_notfound() {
        let w = PatientKey::new("Groth", "Nikolas", "23.02.2001");
        let c = vec![cand("Petersen", "Anna", "01.01.1970", Some("22222"), None, "17000")];
        assert_eq!(resolve_fuzzy(&w, Some("10709"), &c), Resolution::NotFound);
    }

    #[test]
    fn fuzzy_leere_kandidaten_ist_notfound() {
        let w = PatientKey::new("Groth", "Nikolas", "23.02.2001");
        assert_eq!(
            resolve_fuzzy::<String>(&w, None, &[]),
            Resolution::NotFound
        );
    }
}
