//! Fehler der THEDEX-Bibliothek.

use std::path::PathBuf;

pub type Ergebnis<T> = std::result::Result<T, ThedexFehler>;

#[derive(Debug, thiserror::Error)]
pub enum ThedexFehler {
    #[error("Dateizugriff fehlgeschlagen ({pfad}): {quelle}")]
    Io {
        pfad: PathBuf,
        #[source]
        quelle: std::io::Error,
    },

    /// Die Datei ist keine verwertbare THEDEX-Nachricht.
    #[error("keine gültige THEDEX-Nachricht: {0}")]
    Format(String),

    /// Ohne `SenderVerification` (vierstellig, vergeben von SOVDWAER) wird
    /// nichts geschrieben: THEORG verwirft solche Dateien als Irrläufer.
    #[error("SenderVerification fehlt oder ist nicht vierstellig – ohne Kennung von SOVDWAER wird nichts geschrieben")]
    SenderVerificationFehlt,

    #[error("ungültiger Providername {0:?}: 1–8 Zeichen, nur A–Z, 0–9, _ und -")]
    Providername(String),

    #[error("Provider {0:?} hat kein Verzeichnis im THEDEX-Ordner")]
    ZielUnbekannt(String),

    /// Im angegebenen Ordner liegt keine `THEDEX.INI` – vermutlich falscher Pfad.
    #[error("{0} ist kein THEDEX-Ordner (THEDEX.INI fehlt)")]
    KeinThedexOrdner(PathBuf),

    /// Auftrag aus der Cloud passt nicht (fehlende Pflichtfelder, kaputtes Base64 …).
    #[error("Auftrag ungültig: {0}")]
    Auftrag(String),

    /// Patient lässt sich für THEORG nicht eindeutig beschreiben.
    #[error("Patient nicht bestimmbar: {0}")]
    PatientUnbekannt(String),

    #[error("nicht unterstützt: {0}")]
    NichtUnterstuetzt(String),
}

impl ThedexFehler {
    pub(crate) fn io(pfad: impl Into<PathBuf>, quelle: std::io::Error) -> Self {
        ThedexFehler::Io {
            pfad: pfad.into(),
            quelle,
        }
    }

    /// Fehlercode für die Auftragsquittung laut Connector-v2-Vertrag (Abschnitt 6.2).
    pub fn fehlercode(&self) -> &'static str {
        match self {
            ThedexFehler::Io { .. }
            | ThedexFehler::ZielUnbekannt(_)
            | ThedexFehler::KeinThedexOrdner(_) => "pvs_nicht_erreichbar",
            ThedexFehler::Auftrag(_) | ThedexFehler::Format(_) => "ungueltig",
            ThedexFehler::PatientUnbekannt(_) => "patient_unbekannt",
            ThedexFehler::NichtUnterstuetzt(_) => "nicht_unterstuetzt",
            ThedexFehler::SenderVerificationFehlt | ThedexFehler::Providername(_) => "intern",
        }
    }

    /// `true`, wenn ein späterer Versuch Erfolg haben kann (Feld `wiederholbar`).
    pub fn wiederholbar(&self) -> bool {
        matches!(
            self,
            ThedexFehler::Io { .. }
                | ThedexFehler::ZielUnbekannt(_)
                | ThedexFehler::KeinThedexOrdner(_)
        )
    }
}
