# Praxishub Connector

On-Prem-Brücke zwischen Praxishub-Cloud und dem Praxis-PVS (Z1.PRO, charly, …).
Eine Windows-Komponente, drei Use-Cases:

1. **Anamnese-Dokumente** als PDF in die PVS-Patientenakte schreiben (VDDS-media)
2. **HKP-Dokumente** am Patienten ablegen/abrufen (VDDS-media)
3. **HKP-Erkennung** — genehmigte HKPs aus KIM/EBZ erkennen → Auto-Einbestellung

Spezifikation & Hintergrund: **Linear PRA-15**.

## Architektur

Workspace aus zwei Rust-Crates + Vanilla-TS-UI:

```
core/                 # praxishub-connector-core — Tauri-freier, unit-getesteter Logik-Kern
  config / cloud / status / paths
  vdds/  ini.rs        # Selbst-Registrierung als BVS in VDDS_MMI.INI (Windows-1252)
         media.rs      # Dokument-Import in die PVS-Akte (Austausch-INI)
  kim/   pop3.rs        # read-only POP3S-Client (KEIN DELE)
         ebz.rs         # Dienstkennung-Filter EBZ;ANW
         watcher.rs     # nicht-destruktiver Poll-Loop, UIDL-Dedup, Cloud-Meldung
src-tauri/            # praxishub-connector — Desktop-App (UI, Lebenszyklus, Elevation)
src/                 # WebView-Status-/Config-UI
```

**Designprinzipien**
- **Per-User-Install** (asInvoker, wie nelly) — kein Admin. Nur das einmalige
  Schreiben der maschinenweiten `VDDS_MMI.INI` löst per UAC einen kurzen
  Elevation-Schritt aus (`--register-vdds`).
- **KIM-Watcher ist nicht-destruktiv:** kein `DELE`, „leave on server", UIDL-Dedup
  (persistent), Header-Filter `EBZ;ANW`. Eine verlorene EBZ-Genehmigung wäre ein
  Abrechnungsproblem → der PVS-Workflow wird nie gestört.
- **Connector bleibt dumm:** er filtert und liefert die (bereits vom KIM-Clientmodul
  entschlüsselte) Rohnachricht an die Cloud; das CMS/.p7s/XML-Parsing macht die
  Cloud (robust, keine Schema-Rateversuche on-prem).

## Status

Gerüst steht, Frontend baut, der Logik-Kern ist unit-getestet
(`cargo test -p praxishub-connector-core` → 12 grün). Code-Signing-Pipeline
(Azure Trusted Signing via OIDC) ist eingerichtet — siehe
[`docs/SIGNING.md`](docs/SIGNING.md).

**Am Z1-Pilot zu verifizieren (PRA-15):** VDDS-INI-Schema, PDF-Dokumentenablage,
echtes EBZ-`.p7s`-Sample, Per-User-vs-Admin am realen Setup. Siehe Code-Kommentare
mit `verifizieren`/`TODO`.

## Entwicklung

```bash
npm install
npm run tauri dev      # App lokal starten (Windows für VDDS/KIM-Funktion)
cargo test -p praxishub-connector-core
npm run tauri build    # signierter NSIS-Installer (CI: windows-latest)
```

## Background-Betrieb

Der Connector ist ein Daemon: System-Tray-Icon (Menü „Öffnen" / „Beenden"),
Fenster-Schließen minimiert in den Tray (KIM-Watcher läuft weiter), Login-Autostart
startet ihn unsichtbar (`--autostart`). Beenden nur über das Tray-Menü.

## Offene nächste Schritte

- Backend-Routen `/api/v1/connector/{ping,heartbeat,hkp}` sind in der Praxishub-API
  gebaut + auf der Dev-Instanz verifiziert (Prod-Deploy ausstehend). `updates`
  (Self-Update-Feed) noch offen.
- Updater scharf schalten (`createUpdaterArtifacts` + Minisign-Key in `tauri.conf.json`).
- Secrets per Windows-DPAPI statt Klartext-JSON (`core/src/config.rs`).
- VDDS-media: inbound media-Aufruf des PVS behandeln (Connector als Media-Handler).
- First-Run-Onboarding (Tenant/API-Key per Token aus dem Dashboard).
