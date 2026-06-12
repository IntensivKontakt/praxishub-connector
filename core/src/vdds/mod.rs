//! VDDS-media-Baustein.
//!
//! * [`ini`]   — Registrierung von Praxishub als BVS-Modul in `VDDS_MMI.INI`
//! * [`media`] — Ablage von Dokumenten (Anamnese/HKP-PDF) in die PVS-Akte
//!
//! **Wichtig:** VDDS-INIs sind **Windows-1252**-kodiert. Das genaue Schlüssel-
//! Schema (Abschnitte/Keys) ist gegen die VDDS-media-1.4-Spec **und** eine echte
//! `VDDS_MMI.INI` am Z1-Pilot zu verifizieren (PRA-15, Prüfpunkte 1–3).

pub mod ini;
pub mod media;
