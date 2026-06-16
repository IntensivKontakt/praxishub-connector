# Praxishub Connector — Betrieb & Fehlersuche

Operativer Leitfaden: Wer bestimmt den Patienten, wo liegen Dateien, wie sichern
wir das HKP-Abfangen ab, wie sehen wir Fehler sofort, wie wartet man remote.
Status-Marker: ✅ gebaut · 🔧 in Arbeit · 🅿️ am Z1-Pilot zu verifizieren.

---

## 1. Woher weiß Z1 / Praxishub, um welchen Patienten es geht?

**Dokument-Ablage (Anamnese-/HKP-PDF → Akte) — Kaskade:** ✅ (🅿️ am Z1 zu bestätigen)
Das Backend stellt das fertige PDF bereit, der Connector legt es in die Akte. Zur
Patienten-Zuordnung greift eine **Kaskade** (`core/src/documents.rs`):
1. **PATID** — das Backend kennt die Z1-`PATID` in ~90 % der Fälle → direkter,
   unbeaufsichtigter Push über Z1s `MMOINFIMPORT` (`MmoInfIm.exe`). *Variante B.*
2. **Name + Geburtsdatum** — Fallback, wenn keine/abgelehnte PATID.
3. **Variante A (Z1-bestimmt)** — schlägt 1+2 fehl, bleibt das Dokument offen;
   öffnet das Team den Patienten in Z1, übergibt Z1 uns über `PATDATIMPORT` die
   echte `PATID` (`media.rs::handle_invocation`) und wir legen es damit ab.
   **→ Hier bestimmt Z1 den Patienten, nicht wir** — keine Verwechslungsgefahr.
Variante B läuft im KIM-Watcher-Zyklus (`documents::file_pending`), Variante A beim
PVS-Aufruf (`documents::file_pending_for_patient`). 🅿️ Am Z1 zu bestätigen: ob
`MmoInfIm.exe` einen unbeaufsichtigten Push akzeptiert (sonst greift Variante A).

**HKP-Erkennung (KIM/EBZ) — aus der Nachricht selbst:** ✅
Der Patient steht **in der EBZ-Antwort** (KVNR, Name, Geburtsdatum im signierten
KZBV-XML). Die Cloud parst das und matcht auf den `CrmPatient` (Name+Geburtsdatum,
KVNR), pro Tenant. Kein Z1 nötig. Bei Mehrdeutigkeit → **Team-Aufgabe** statt
automatischer Fehlzuordnung.

---

## 2. Archiv-/Austausch-Pfad: Default, prüfen, ändern

Zwei Pfade NICHT verwechseln:

| Pfad | Wer besitzt ihn | Konfigurierbar? |
|---|---|---|
| **PVS-Archiv** (wo Z1 das PDF endgültig ablegt, CGM PRAXISARCHIV) | **Z1** | nein (Z1-Sache; wir schreiben da nicht direkt rein, wir übergeben das PDF via media, Z1 importiert) |
| **Austausch-Verzeichnis** (wo der Connector die temporäre `VDDS_MMO.INI` + das PDF ablegt) | Connector | **ja** — `exchange_dir` in der Config 🔧 |

**Default:** leer → Windows-Temp (`%TEMP%`).
**Prüfen:** Connector-Fenster → Konfiguration → „Austausch-Verzeichnis"; bzw.
`config.json` im Per-User-AppData (`%APPDATA%\ai.praxishub.connector\config.json`).
**Ändern:** Feld setzen und speichern (oder `config.json` editieren). 🅿️ Am Z1
prüfen, ob Z1 ein **festes Abholverzeichnis** erwartet — dann dieses eintragen.

---

## 3. Wie stellen wir sicher, dass das HKP-Abfangen klappt?

**Eingebaute Schutzmechanismen** ✅
- **Nicht-destruktiv:** read-only POP3, **kein `DELE`**, „leave on server", UIDL-Dedup
  → der Connector kann dem PVS niemals eine EBZ-Mail wegnehmen.
- **Filter:** nur `X-KIM-Dienstkennung: EBZ;ANW` (genehmigte HKPs, DSGVO-minimal).
- **Idempotent:** dieselbe UIDL wird nie doppelt gemeldet (Backend dedupt zusätzlich).

**Verifizieren**
- Connector-Fenster zeigt **KIM-Status** (verbunden, letzter Poll, letzte erkannte HKP). ✅
- **„KIM testen"**-Button: Verbindung + Postfach-Zähler. ✅
- **Heartbeat** an die Cloud: `kim_watching` + letzter Fehler. ✅ (last-poll/last-hkp 🔧)
- 🅿️ **End-to-End am Pilot:** einen echten genehmigten EBZ-Datensatz durchlaufen
  lassen → im Praxishub eine HKP-Meldung + Team-Aufgabe „Patient einbestellen".

**Voraussetzungen am Pilot** 🅿️
- Im KIM-Clientmodul **Aufbewahrung / „leave on server"** aktiv (sonst löscht das
  PVS die Mail vor uns).
- Postfach-Zugangsdaten im Connector hinterlegt (First-Run-Wizard).

---

## 4. Wie sehen wir sofort, wenn etwas nicht klappt / der Dienst abbricht?

- **Heartbeat (Connector → Cloud, ~60 s):** Status + letzter Fehler. ✅/🔧
- **Watchdog (Backend):** Alarm via **ntfy** (bestehende Error-Monitoring-Infra),
  wenn ein Connector **seit > N Minuten keinen Heartbeat** sendet (Rechner aus /
  Dienst tot) **oder einen Fehler meldet**. Optional Systemnachricht im
  Praxis-Posteingang. 🔧
- **Dashboard:** Connector-Status (letzter Kontakt, VDDS/KIM/Cloud, letzter Fehler)
  in den Einstellungen. 🔧
- **Lokal:** Der Connector schreibt ein Log (`tracing`) unter
  `%APPDATA%\ai.praxishub.connector\...\logs`. ✅

---

## 5. Wie kann ich den Connector remote warten?

- **Auto-Update:** ✅ signierte Releases (Git-Tag `v*`) → Connectoren aktualisieren
  sich selbst über den Updater-Feed (`/api/v1/connector/updates/...`). Code-Wartung
  ohne Vor-Ort-Zugriff. (Feed-Befüllung via `CONNECTOR_UPDATE_MANIFEST` 🔧)
- **Health-Überblick:** ✅ Heartbeat zeigt zentral, welche Praxis-Connectoren laufen
  bzw. Probleme haben.
- **Config:** aktuell per-Praxis lokal (Connector-Fenster / `config.json`).
  Remote-Config-Push = möglicher Ausbau.
- **Grenzen:** Remote-Restart/-Shell ist **nicht** eingebaut (bräuchte einen
  Command-Kanal Cloud→Connector). Auto-Update + Heartbeat + Fehleralarm decken den
  Normalbetrieb ab; bei hartem Ausfall hilft der Watchdog-Alarm → gezielt die Praxis
  kontaktieren.

---

## Schnell-Checkliste bei „es geht nicht"
1. Connector-Fenster offen? Tray-Icon vorhanden? (sonst läuft der Dienst nicht)
2. Cloud-Status grün? (sonst API-Key/URL prüfen — First-Run-Wizard)
3. KIM-Status grün? „KIM testen" → Postfach erreichbar? Zugangsdaten/Port korrekt?
4. VDDS registriert? (PVS-Registrierung im Fenster, einmalig Admin)
5. Watchdog-Alarm bekommen? → Heartbeat-Lücke = Rechner/Dienst aus.
6. Logs im AppData prüfen.
