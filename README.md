# Praxishub Connector

On-Prem-Brücke zwischen Praxishub-Cloud und dem Praxis-PVS (Z1.PRO, charly, …).
Eine Windows-Komponente, drei Use-Cases:

1. **Anamnese-Dokumente** als PDF in die PVS-Patientenakte schreiben (VDDS-media)
2. **HKP-Dokumente** am Patienten ablegen/abrufen (VDDS-media)
3. **HKP-Erkennung** — genehmigte HKPs aus KIM/EBZ erkennen → Auto-Einbestellung

Spezifikation & Hintergrund: **Linear PRA-15**.

## Status
Plumbing-Phase. Code-Signing-Pipeline (Azure Trusted/Artifact Signing via GitHub
OIDC) ist eingerichtet — siehe [`docs/SIGNING.md`](docs/SIGNING.md). Tauri-App-Gerüst
folgt.

## Geplanter Stack
Tauri (Rust-Core + WebView-Status-UI), NSIS-Installer, Windows-Service für den
KIM-Watcher. Referenz: samedi- & nelly-Connector (an PRA-15 angehängt).
