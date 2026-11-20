//! THEDEX-Ordner: Providerliste, Selbsteintrag, eigener Eingang, Versand.
//!
//! Aufbau (Spezifikation Kapitel 1.4 und 5):
//!
//! ```text
//! <THEDEX>\THEDEX.INI          [THEDEX-Providers]  NAME=Beschreibung
//! <THEDEX>\THEORG\             Eingang von THEORG
//! <THEDEX>\PRAXHUB\            unser Eingang
//! <THEDEX>\PRAXHUB\FEHLER\     nicht lesbare Dateien (statt sie zu löschen)
//! ```
//!
//! Jeder Provider liest nur seinen eigenen Eingang und löscht dort, was er
//! verarbeitet hat. Wer an einen anderen Provider sendet, legt die Datei in
//! dessen Eingang.

use crate::ausgang::{verification_gueltig, Anhang};
use crate::fehler::{Ergebnis, ThedexFehler};
use crate::modell::{Berichtsdaten, Nachricht};
use crate::zeichensatz::Zeichensatz;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Name der gemeinsamen Konfigurationsdatei.
pub const INI_NAME: &str = "THEDEX.INI";
/// Abschnitt mit einer Zeile je Provider.
pub const PROVIDER_ABSCHNITT: &str = "THEDEX-Providers";
/// Unterordner des eigenen Eingangs für nicht lesbare Dateien.
pub const FEHLER_ORDNER: &str = "FEHLER";

/// Providername prüfen (Verzeichnisname nach 8+0: 1–8 Zeichen `A–Z 0–9 _ -`)
/// und in Großschreibung zurückgeben.
pub fn providername_pruefen(name: &str) -> Ergebnis<String> {
    let n = name.trim();
    let ok = !n.is_empty()
        && n.len() <= 8
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(n.to_ascii_uppercase())
    } else {
        Err(ThedexFehler::Providername(name.to_string()))
    }
}

/// Eintrag aus `[THEDEX-Providers]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderEintrag {
    pub name: String,
    pub beschreibung: String,
    /// Vorhandenes Verzeichnis des Providers (oder `None`, wenn es fehlt).
    pub verzeichnis: Option<PathBuf>,
}

/// Ergebnis des Selbsteintrags in die `THEDEX.INI`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Selbsteintrag {
    Neu,
    Aktualisiert,
    Unveraendert,
}

/// Eine Datei aus einem Eingang.
#[derive(Debug, Clone)]
pub enum Eingang {
    Lesbar(Box<EingangsDatei>),
    /// Nicht als THEDEX-Nachricht lesbar.
    Defekt {
        pfad: PathBuf,
        fehler: String,
    },
}

/// Gelesene Nachricht samt Pfad und (falls vorhanden) Anhangsdatei.
#[derive(Debug, Clone)]
pub struct EingangsDatei {
    pub pfad: PathBuf,
    pub nachricht: Nachricht,
    /// `ReportFile`, sofern er im selben Ordner liegt.
    pub anhang: Option<PathBuf>,
}

/// Ergebnis von [`ThedexOrdner::verarbeiten`].
#[derive(Debug, Clone, Default)]
pub struct Verarbeitungsbericht {
    /// Übergeben und gelöscht.
    pub uebergeben: Vec<PathBuf>,
    /// Übergabe fehlgeschlagen; Datei bleibt liegen (mit Fehlertext). Danach
    /// wird abgebrochen, damit die Reihenfolge der Änderungen erhalten bleibt.
    pub zurueckgestellt: Option<(PathBuf, String)>,
    /// In `FEHLER` verschoben (neuer Pfad, Grund).
    pub defekt: Vec<(PathBuf, String)>,
}

/// Pfade einer gesendeten Nachricht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gesendet {
    pub tdx: PathBuf,
    pub anhang: Option<PathBuf>,
}

// ------------------------------------------------------------- THEDEX.INI

/// Pfad der `THEDEX.INI` (Schreibweise egal), falls vorhanden.
pub fn ini_finden(wurzel: &Path) -> Option<PathBuf> {
    eintrag_finden(wurzel, INI_NAME).filter(|p| p.is_file())
}

/// Liest `[THEDEX-Providers]`. Nur lesend.
pub fn providers_lesen(wurzel: &Path) -> Ergebnis<Vec<ProviderEintrag>> {
    let pfad = ini_finden(wurzel).ok_or_else(|| ThedexFehler::KeinThedexOrdner(wurzel.into()))?;
    let bytes = fs::read(&pfad).map_err(|e| ThedexFehler::io(&pfad, e))?;
    let z = crate::ini::zerlegen(&Zeichensatz::Ansi.dekodieren(&bytes));
    Ok(z.abschnitte
        .iter()
        .filter(|a| a.name.eq_ignore_ascii_case(PROVIDER_ABSCHNITT))
        .flat_map(|a| a.felder.iter())
        .map(|f| ProviderEintrag {
            name: f.schluessel.clone(),
            beschreibung: f.wert.clone(),
            verzeichnis: eintrag_finden(wurzel, &f.schluessel).filter(|p| p.is_dir()),
        })
        .collect())
}

/// Setzt die eigene Zeile `PROVIDER=Beschreibung` in `[THEDEX-Providers]`.
///
/// Arbeitet auf den Bytes der Datei: Alle anderen Zeilen, Abschnitte,
/// Zeilenenden und der Zeichensatz bleiben unverändert. Doppelte eigene
/// Zeilen werden entfernt (die Spezifikation erlaubt genau eine). Die
/// Beschreibung wird als ANSI geschrieben. `None` = nichts zu ändern.
pub fn ini_eintrag_setzen(
    datei: &[u8],
    provider: &str,
    beschreibung: &str,
) -> Option<(Vec<u8>, Selbsteintrag)> {
    let zeilen = zeilen_mit_ende(datei);
    let eol: &[u8] = if datei.windows(2).any(|w| w == b"\r\n") || !datei.contains(&b'\n') {
        b"\r\n"
    } else {
        b"\n"
    };
    let beschreibung = beschreibung.replace(['\r', '\n'], " ");
    let mut neue_zeile = provider.as_bytes().to_vec();
    neue_zeile.push(b'=');
    neue_zeile.extend(Zeichensatz::Ansi.kodieren(beschreibung.trim()));

    let mut im_abschnitt = false;
    let mut abschnitt_gefunden = false;
    let mut letzte_inhaltszeile: Option<usize> = None;
    let mut eigene: Vec<usize> = Vec::new();
    for (i, z) in zeilen.iter().enumerate() {
        let inhalt = trim_bytes(z);
        if inhalt.first() == Some(&b'[') && inhalt.contains(&b']') {
            let ende = inhalt
                .iter()
                .position(|&b| b == b']')
                .unwrap_or(inhalt.len());
            let name = trim_bytes(&inhalt[1..ende]);
            im_abschnitt = name.eq_ignore_ascii_case(PROVIDER_ABSCHNITT.as_bytes());
            if im_abschnitt {
                abschnitt_gefunden = true;
                letzte_inhaltszeile = Some(i);
            }
            continue;
        }
        if !im_abschnitt || inhalt.is_empty() {
            continue;
        }
        letzte_inhaltszeile = Some(i);
        if let Some(pos) = inhalt.iter().position(|&b| b == b'=') {
            if trim_bytes(&inhalt[..pos]).eq_ignore_ascii_case(provider.as_bytes()) {
                eigene.push(i);
            }
        }
    }

    let mut out: Vec<Vec<u8>> = zeilen.iter().map(|z| z.to_vec()).collect();
    let ergebnis;
    if let Some((&erste, rest)) = eigene.split_first() {
        let alt = trim_bytes(&out[erste]);
        let pos = alt.iter().position(|&b| b == b'=').unwrap_or(0);
        let alter_wert = trim_bytes(&alt[pos + 1..]).to_vec();
        let neuer_wert = &neue_zeile[provider.len() + 1..];
        if rest.is_empty() && alter_wert == neuer_wert {
            return None;
        }
        if alter_wert != neuer_wert {
            let ende = zeilenende(&out[erste]).to_vec();
            out[erste] = [neue_zeile.as_slice(), ende.as_slice()].concat();
        }
        for &i in rest.iter().rev() {
            out.remove(i);
        }
        ergebnis = Selbsteintrag::Aktualisiert;
    } else if abschnitt_gefunden {
        let nach = letzte_inhaltszeile.unwrap_or(0);
        if zeilenende(&out[nach]).is_empty() {
            out[nach].extend_from_slice(eol);
        }
        out.insert(nach + 1, [neue_zeile.as_slice(), eol].concat());
        ergebnis = Selbsteintrag::Neu;
    } else {
        if let Some(letzte) = out.last_mut() {
            if zeilenende(letzte).is_empty() {
                letzte.extend_from_slice(eol);
            }
        }
        if out.last().is_some_and(|z| !trim_bytes(z).is_empty()) {
            out.push(eol.to_vec());
        }
        out.push([b"[", PROVIDER_ABSCHNITT.as_bytes(), b"]", eol].concat());
        out.push([neue_zeile.as_slice(), eol].concat());
        ergebnis = Selbsteintrag::Neu;
    }
    Some((out.concat(), ergebnis))
}

fn zeilen_mit_ende(bytes: &[u8]) -> Vec<&[u8]> {
    let mut v = Vec::new();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            v.push(&bytes[start..=i]);
            start = i + 1;
        }
    }
    if start < bytes.len() {
        v.push(&bytes[start..]);
    }
    v
}

fn zeilenende(z: &[u8]) -> &[u8] {
    if z.ends_with(b"\r\n") {
        &z[z.len() - 2..]
    } else if z.ends_with(b"\n") {
        &z[z.len() - 1..]
    } else {
        &[]
    }
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    let anfang = b
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    let ende = b
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(anfang);
    &b[anfang..ende.max(anfang)]
}

// ------------------------------------------------------------- Eingang lesen

/// Liest alle `.TDX` eines Ordners (nur lesend), älteste zuerst. Dateien, die
/// jünger als `mindestalter` sind, werden übergangen – sie werden womöglich
/// gerade noch geschrieben.
pub fn ordner_lesen(ordner: &Path, mindestalter: Duration) -> Ergebnis<Vec<Eingang>> {
    let mut dateien: Vec<(SystemTime, PathBuf)> = Vec::new();
    let liste = fs::read_dir(ordner).map_err(|e| ThedexFehler::io(ordner, e))?;
    let jetzt = SystemTime::now();
    for eintrag in liste {
        let eintrag = eintrag.map_err(|e| ThedexFehler::io(ordner, e))?;
        let pfad = eintrag.path();
        let ist_tdx = pfad
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("tdx"));
        if !ist_tdx {
            continue;
        }
        let meta = eintrag.metadata().map_err(|e| ThedexFehler::io(&pfad, e))?;
        if !meta.is_file() {
            continue;
        }
        let geaendert = meta.modified().unwrap_or(UNIX_EPOCH);
        if !mindestalter.is_zero()
            && jetzt.duration_since(geaendert).unwrap_or_default() < mindestalter
        {
            continue;
        }
        dateien.push((geaendert, pfad));
    }
    dateien.sort();

    let mut ergebnis = Vec::with_capacity(dateien.len());
    for (_, pfad) in dateien {
        let bytes = match fs::read(&pfad) {
            Ok(b) => b,
            // In der Zwischenzeit von jemand anderem entfernt: überspringen.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(ThedexFehler::io(&pfad, e)),
        };
        match Nachricht::aus_bytes(&bytes) {
            Ok(nachricht) => {
                let anhang = nachricht
                    .report_data
                    .as_ref()
                    .and_then(|r| r.report_file.as_deref())
                    .and_then(|f| anhang_finden(ordner, f));
                ergebnis.push(Eingang::Lesbar(Box::new(EingangsDatei {
                    pfad,
                    nachricht,
                    anhang,
                })));
            }
            Err(e) => ergebnis.push(Eingang::Defekt {
                pfad,
                fehler: e.to_string(),
            }),
        }
    }
    Ok(ergebnis)
}

/// `ReportFile` im selben Ordner suchen; Pfadanteile werden ignoriert.
fn anhang_finden(ordner: &Path, report_file: &str) -> Option<PathBuf> {
    let name = report_file.trim().rsplit(['\\', '/']).next()?.trim();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    eintrag_finden(ordner, name).filter(|p| p.is_file())
}

/// Eintrag eines Ordners ohne Rücksicht auf Groß-/Kleinschreibung suchen.
fn eintrag_finden(ordner: &Path, name: &str) -> Option<PathBuf> {
    let direkt = ordner.join(name);
    if direkt.exists() {
        return Some(direkt);
    }
    fs::read_dir(ordner)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

// ------------------------------------------------------------- ThedexOrdner

/// Ein THEDEX-Ordner aus Sicht eines Providers (Standard `PRAXHUB`).
#[derive(Debug, Clone)]
pub struct ThedexOrdner {
    wurzel: PathBuf,
    provider: String,
    /// Dateien im Eingang, die jünger sind, bleiben bis zum nächsten Lauf liegen.
    pub mindestalter: Duration,
}

impl ThedexOrdner {
    pub fn neu(wurzel: impl Into<PathBuf>, provider: &str) -> Ergebnis<Self> {
        Ok(ThedexOrdner {
            wurzel: wurzel.into(),
            provider: providername_pruefen(provider)?,
            mindestalter: Duration::from_secs(2),
        })
    }

    pub fn wurzel(&self) -> &Path {
        &self.wurzel
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Eigener Eingang `<THEDEX>\<PROVIDER>`.
    pub fn eingang(&self) -> PathBuf {
        eintrag_finden(&self.wurzel, &self.provider)
            .unwrap_or_else(|| self.wurzel.join(&self.provider))
    }

    /// `<THEDEX>\<PROVIDER>\FEHLER`
    pub fn fehler_ordner(&self) -> PathBuf {
        self.eingang().join(FEHLER_ORDNER)
    }

    /// Eigenes Verzeichnis anlegen und Zeile in `THEDEX.INI` setzen.
    /// Verlangt eine vorhandene `THEDEX.INI` (sonst ist der Pfad vermutlich falsch).
    pub fn einrichten(&self, beschreibung: &str) -> Ergebnis<Selbsteintrag> {
        let ini = ini_finden(&self.wurzel)
            .ok_or_else(|| ThedexFehler::KeinThedexOrdner(self.wurzel.clone()))?;
        let eingang = self.eingang();
        fs::create_dir_all(&eingang).map_err(|e| ThedexFehler::io(&eingang, e))?;
        let alt = fs::read(&ini).map_err(|e| ThedexFehler::io(&ini, e))?;
        let Some((neu, ergebnis)) = ini_eintrag_setzen(&alt, &self.provider, beschreibung) else {
            return Ok(Selbsteintrag::Unveraendert);
        };
        let tmp = self.wurzel.join(format!("{}.TMP", &self.provider));
        atomar_schreiben(&tmp, &ini, &neu)?;
        Ok(ergebnis)
    }

    pub fn providers(&self) -> Ergebnis<Vec<ProviderEintrag>> {
        providers_lesen(&self.wurzel)
    }

    /// Eigenen Eingang lesen (nur lesend; nichts wird gelöscht oder verschoben).
    pub fn eingang_lesen(&self) -> Ergebnis<Vec<Eingang>> {
        let eingang = self.eingang();
        if !eingang.is_dir() {
            return Ok(Vec::new());
        }
        ordner_lesen(&eingang, self.mindestalter)
    }

    /// Nach erfolgreicher Übergabe (Cloud-Quittung): Datei und Anhang löschen.
    pub fn quittieren(&self, datei: &EingangsDatei) -> Ergebnis<()> {
        loeschen(&datei.pfad)?;
        if let Some(a) = &datei.anhang {
            loeschen(a)?;
        }
        Ok(())
    }

    /// Nicht lesbare Datei nach `FEHLER` verschieben und den Grund daneben
    /// ablegen (`<Name>.TXT`). Gibt den neuen Pfad zurück.
    pub fn als_defekt_ablegen(&self, pfad: &Path, grund: &str) -> Ergebnis<PathBuf> {
        let ordner = self.fehler_ordner();
        fs::create_dir_all(&ordner).map_err(|e| ThedexFehler::io(&ordner, e))?;
        let name = pfad
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "DATEI.TDX".into());
        let mut ziel = ordner.join(&name);
        let mut nr = 1;
        while ziel.exists() {
            ziel = ordner.join(format!("{nr}_{name}"));
            nr += 1;
        }
        fs::rename(pfad, &ziel).map_err(|e| ThedexFehler::io(pfad, e))?;
        let mut grund_datei = ziel.clone().into_os_string();
        grund_datei.push(".TXT");
        // Der Grund ist nur eine Lesehilfe; scheitert das Schreiben, bleibt die Datei trotzdem in FEHLER.
        let _ = fs::write(&grund_datei, Zeichensatz::Ansi.kodieren(grund));
        Ok(ziel)
    }

    /// Eingang abarbeiten: `uebergeben` bekommt jede lesbare Nachricht (älteste
    /// zuerst). `Ok` → Datei wird gelöscht; `Err` → Datei bleibt liegen und die
    /// Runde endet (spätere Änderungen dürfen frühere nicht überholen). Defekte
    /// Dateien wandern nach `FEHLER`.
    pub fn verarbeiten<F, E>(&self, mut uebergeben: F) -> Ergebnis<Verarbeitungsbericht>
    where
        F: FnMut(&EingangsDatei) -> Result<(), E>,
        E: std::fmt::Display,
    {
        let mut bericht = Verarbeitungsbericht::default();
        for eingang in self.eingang_lesen()? {
            match eingang {
                Eingang::Defekt { pfad, fehler } => {
                    let neu = self.als_defekt_ablegen(&pfad, &fehler)?;
                    bericht.defekt.push((neu, fehler));
                }
                Eingang::Lesbar(datei) => match uebergeben(&datei) {
                    Ok(()) => {
                        self.quittieren(&datei)?;
                        bericht.uebergeben.push(datei.pfad);
                    }
                    Err(e) => {
                        bericht.zurueckgestellt = Some((datei.pfad, e.to_string()));
                        break;
                    }
                },
            }
        }
        Ok(bericht)
    }

    /// Nachricht in den Eingang von `ziel` schreiben (z. B. `THEORG`).
    ///
    /// Reihenfolge: Anhang (falls vorhanden) unter temporärem Namen schreiben
    /// und umbenennen, dann die `.TDX` ebenso. THEORG sieht eine `.TDX` also
    /// erst, wenn sie und ihr Anhang vollständig sind. Ohne gültige
    /// `SenderVerification` wird nichts geschrieben.
    pub fn senden(
        &self,
        ziel: &str,
        nachricht: &mut Nachricht,
        anhang: Option<&Anhang>,
    ) -> Ergebnis<Gesendet> {
        let verification_ok = nachricht
            .sender
            .as_ref()
            .and_then(|s| s.verification.as_deref())
            .is_some_and(verification_gueltig);
        if !verification_ok {
            return Err(ThedexFehler::SenderVerificationFehlt);
        }
        let ziel_name = providername_pruefen(ziel)?;
        let ordner = eintrag_finden(&self.wurzel, &ziel_name)
            .filter(|p| p.is_dir())
            .ok_or_else(|| ThedexFehler::ZielUnbekannt(ziel_name.clone()))?;

        let endung = anhang.map(|a| endung_pruefen(&a.endung)).transpose()?;
        let mut belegt = vec!["TDX", "TMP", "TMA"];
        if let Some(e) = &endung {
            belegt.push(e.as_str());
        }
        let stamm = freier_stamm(&ordner, &belegt);

        let mut anhang_pfad = None;
        if let (Some(a), Some(e)) = (anhang, &endung) {
            let name = format!("{stamm}.{e}");
            let bericht = nachricht
                .report_data
                .get_or_insert_with(Berichtsdaten::default);
            bericht.report_file = Some(name.clone());
            if bericht
                .report_type
                .as_deref()
                .is_none_or(|t| t.trim().is_empty())
            {
                bericht.report_type = Some(e.clone());
            }
            let ziel_pfad = ordner.join(&name);
            atomar_schreiben(&ordner.join(format!("{stamm}.TMA")), &ziel_pfad, &a.inhalt)?;
            anhang_pfad = Some(ziel_pfad);
        }

        let tdx = ordner.join(format!("{stamm}.TDX"));
        if let Err(e) = atomar_schreiben(
            &ordner.join(format!("{stamm}.TMP")),
            &tdx,
            &nachricht.als_bytes(),
        ) {
            if let Some(a) = &anhang_pfad {
                let _ = fs::remove_file(a);
            }
            return Err(e);
        }
        Ok(Gesendet {
            tdx,
            anhang: anhang_pfad,
        })
    }
}

/// Dateiendung eines Anhangs: 1–3 Zeichen A–Z/0–9 (8.3).
fn endung_pruefen(e: &str) -> Ergebnis<String> {
    let e = e.trim().trim_start_matches('.').to_ascii_uppercase();
    if (1..=3).contains(&e.len())
        && e.bytes().all(|b| b.is_ascii_alphanumeric())
        && !["TDX", "TMP", "TMA"].contains(&e.as_str())
    {
        Ok(e)
    } else {
        Err(ThedexFehler::NichtUnterstuetzt(format!(
            "Anhangsendung {e:?}"
        )))
    }
}

/// Achtstelliger Dateistamm `PH` + 6 Zeichen (0–9, A–Z), der im Ordner mit
/// keiner der Endungen belegt ist.
fn freier_stamm(ordner: &Path, endungen: &[&str]) -> String {
    static ZAEHLER: AtomicU64 = AtomicU64::new(0);
    const ZEICHEN: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    loop {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let z = ZAEHLER.fetch_add(1, Ordering::Relaxed);
        let mut x =
            nanos ^ (u64::from(std::process::id()) << 32) ^ z.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        // Durchmischen (splitmix64), damit aufeinanderfolgende Namen nicht nur in der letzten Stelle abweichen.
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        let mut stamm = String::from("PH");
        for _ in 0..6 {
            stamm.push(ZEICHEN[(x % 36) as usize] as char);
            x /= 36;
        }
        let frei = endungen
            .iter()
            .all(|e| eintrag_finden(ordner, &format!("{stamm}.{e}")).is_none());
        if frei {
            return stamm;
        }
    }
}

/// Schreibt `inhalt` nach `tmp`, synchronisiert und benennt in `ziel` um.
fn atomar_schreiben(tmp: &Path, ziel: &Path, inhalt: &[u8]) -> Ergebnis<()> {
    let schreiben = || -> std::io::Result<()> {
        let mut f = fs::File::create(tmp)?;
        f.write_all(inhalt)?;
        f.sync_all()?;
        drop(f);
        fs::rename(tmp, ziel)
    };
    schreiben().map_err(|e| {
        let _ = fs::remove_file(tmp);
        ThedexFehler::io(ziel, e)
    })
}

fn loeschen(pfad: &Path) -> Ergebnis<()> {
    match fs::remove_file(pfad) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ThedexFehler::io(pfad, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providernamen() {
        assert_eq!(providername_pruefen("praxhub").unwrap(), "PRAXHUB");
        assert!(providername_pruefen("PRAXISHUB").is_err());
        assert!(providername_pruefen("").is_err());
        assert!(providername_pruefen("A/B").is_err());
    }

    #[test]
    fn ini_neu_im_vorhandenen_abschnitt() {
        let alt = b"[THEDEX-Providers]\r\nTHEORG=Theorg\r\n\r\n[THEORG_PROXOS]\r\nExport=1\r\n";
        let (neu, e) = ini_eintrag_setzen(alt, "PRAXHUB", "Praxishub").unwrap();
        assert_eq!(e, Selbsteintrag::Neu);
        assert_eq!(
            neu,
            b"[THEDEX-Providers]\r\nTHEORG=Theorg\r\nPRAXHUB=Praxishub\r\n\r\n[THEORG_PROXOS]\r\nExport=1\r\n"
        );
        assert!(ini_eintrag_setzen(&neu, "PRAXHUB", "Praxishub").is_none());
    }

    #[test]
    fn ini_ohne_abschnitt_lf_und_ohne_schlusszeile() {
        let (neu, _) = ini_eintrag_setzen(b"[Andere]\nx=1", "PRAXHUB", "P").unwrap();
        assert_eq!(neu, b"[Andere]\nx=1\n\n[THEDEX-Providers]\nPRAXHUB=P\n");
        let (leer, _) = ini_eintrag_setzen(b"", "PRAXHUB", "P").unwrap();
        assert_eq!(leer, b"[THEDEX-Providers]\r\nPRAXHUB=P\r\n");
    }

    #[test]
    fn ini_aktualisieren_und_duplikate_entfernen() {
        let alt = b"[thedex-providers]\r\npraxhub = Alt\r\nTHEORG=T\r\nPRAXHUB=Doppelt\r\n";
        let (neu, e) = ini_eintrag_setzen(alt, "PRAXHUB", "Neu").unwrap();
        assert_eq!(e, Selbsteintrag::Aktualisiert);
        assert_eq!(neu, b"[thedex-providers]\r\nPRAXHUB=Neu\r\nTHEORG=T\r\n");
    }

    #[test]
    fn freie_staemme_sind_8_3() {
        let dir = std::env::temp_dir();
        let s = freier_stamm(&dir, &["TDX"]);
        assert_eq!(s.len(), 8);
        assert!(s.starts_with("PH"));
        assert_ne!(s, freier_stamm(&dir, &["TDX"]));
    }
}
