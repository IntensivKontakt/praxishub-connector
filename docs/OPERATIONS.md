# Praxishub Connector — Betrieb & Fehlersuche

Operativer Leitfaden: Wer bestimmt den Patienten, wo liegen Dateien, wie sichern
wir das HKP-Abfangen ab, wie sehen wir Fehler sofort, wie wartet man remote.
Status-Marker: ✅ gebaut · 🔧 in Arbeit · 🅿️ am Z1-Pilot zu verifizieren.

---

## 1. Woher weiß Z1 / Praxishub, um welchen Patienten es geht?

**Dokument-Ablage (Anamnese-/HKP-PDF → Akte) — PVS-initiiert:** ✅
Der Anwender hat in Z1 einen Patienten geöffnet und löst die Praxishub-Aktion aus.
Z1 schreibt dann den **aktuellen Patientenkontext** (`PATID`, Name, Geburtsdatum)
in die Austausch-INI (`VDDS_MMO.INI`, Sektion `[PATIENT]`) und ruft unser
registriertes Modul auf. Der Connector liest diesen Kontext
(`core/src/vdds/media.rs::handle_invocation`). **→ Z1 bestimmt den Patienten, nicht wir.**
Das ist der sichere Weg: keine Patienten-Auswahl/Verwechslung auf unserer Seite.

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
| **PVS-Importprogramm** (VDDS-media-Import-.exe des PVS, das der Connector zum Ablegen aufruft) | Z1, im `VDDS_MMI.INI` registriert | **ja** — `pvs_import_program` in der Config 🔧 |

**Default:** leer → Windows-Temp (`%TEMP%`).
**Prüfen:** Connector-Fenster → Konfiguration → „Austausch-Verzeichnis"; bzw.
`config.json` im Per-User-AppData (`%APPDATA%\ai.praxishub.connector\config.json`).
**Ändern:** Feld setzen und speichern (oder `config.json` editieren). 🅿️ Am Z1
prüfen, ob Z1 ein **festes Abholverzeichnis** erwartet — dann dieses eintragen.

> **Dokument-Ablage (Anamnese/HKP-PDF → Z1-Akte):** Der Connector pollt
> `GET /connector/documents/pending`, legt jedes PDF per VDDS-media über das in
> **`pvs_import_program`** hinterlegte Z1-Importprogramm ab und quittiert
> `…/filed` (mit der getroffenen **Z1-PATID**) bzw. `…/failed` (Grund). Ohne
> gesetztes `pvs_import_program` **pausiert** die Ablage (Dokumente bleiben
> „wartet auf Übertragung"). Dokumente **ohne** Z1-PATID kann der Connector nicht
> eindeutig zuordnen → sie werden als „Nicht zugeordnet" gemeldet; die Praxis
> trägt dann die Patientennummer nach und stößt die Ablage erneut an.

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
