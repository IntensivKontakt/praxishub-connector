# THEORG-Connector: Plan für Dienst-Modus und Firebird-Leseadapter

Stand: 23.11.2026. Anlass: Die Physiopraxis Storhas (THEORG in der IDT Cloud) hat bei IDT angefragt, ob der
Connector auf dem Cloud-Server laufen darf. In der Anfrage stehen „läuft als Windows-Dienst“ und „liest aus der
THEORG-Datenbank“. Beides gibt es noch nicht. Dieser Plan beschreibt, was bis zur Installation fehlt, in welcher
Reihenfolge und was wovon abhängt. Für IDT gibt es im Walletpass-Repo ein Technikblatt
(`docs/plans/fachrichtungen/sichtung/S1-idt-technikblatt.md`). Was dort *[geplant]* heißt, steht hier.

Maßgeblich für die Cloud-Seite ist der Connector-v2-Vertrag (Walletpass `docs/connector-v2-vertrag.md`, Modelle
in `backend/app/connector_v2/vertrag.py`).

## 1. Stand im Repo

| Baustein | Stand | Ort |
|---|---|---|
| Z1-Connector (Desktop-App, Tray, Autostart, Per-User-Install) | im Einsatz, spricht Vertrag v1 | `core/`, `src-tauri/` |
| Signatur (Authenticode über Azure Trusted Signing), Updater mit Minisign | fertig | `docs/SIGNING.md`, `.github/workflows/build-sign.yml` |
| THEDEX-Bibliothek mit Abbildung auf v2 (`PatientV2`, `MitgliedschaftV2`, `CheckinV2`) | fertig, ohne Netz | `theorg/thedex/` |
| Nachbildung der THEORG-Datenbank ohne Patientendaten | fertig (Beispielschema) | `theorg/db/` |
| THEORG-Kartierung (Tabellen, Codes, Schreibwege) | Vorlage, leer | `docs/THEORG-SCHEMA.md` |
| v2-Client (`hello`, `heartbeat`, `ingest`, `jobs`, `ergebnis`) | fehlt | – |
| Dienst-Modus | fehlt | – |
| Firebird-Zugriff | fehlt | – |
| Update-Kanal `theorg` | fehlt (nur `z1` in `UPDATE_CHANNELS`) | Walletpass `backend/app/services/connector_update.py` |

## 2. Zielbild

Eine eigene App `praxishub-theorg` (App-Kennung und Update-Kanal `theorg`, Tags `theorg-v<semver>`). Sie ist ein
Windows-Dienst ohne Oberfläche und damit ohne WebView2. Die Tauri-App bleibt für Z1 unverändert.

```
theorg/
  thedex/      vorhanden: THEDEX-Dateien lesen/schreiben, Abbildung auf v2
  db/          vorhanden: Nachbildung, Beispielschema
  fbdb/        neu: Firebird-Verbindung, Schema-Fingerabdruck, Leser je Entität (später Schreiber)
  dienst/      neu: Binary praxishub-theorg (Windows-Dienst + Kommandozeile), Zyklus, Konfiguration
connector-v2/  neu: v2-Client, Typen aus dem Vertrag, Idempotenz-Speicher (auch für spätere PVS-Adapter)
```

Wiederverwenden aus `core`: `crypto` (DPAPI), `error`. `paths` bekommt eine Variante für
`C:\ProgramData\Praxishub\Connector`.

## 3. Arbeitspakete

### A. v2-Client (`connector-v2`)

Typen für `HelloIn/Out`, `HeartbeatIn/Out`, `IngestIn/Out`, `JobOut`, `JobErgebnisIn` und die Entitäten. Die
v2-Typen aus `theorg/thedex/src/v2.rs` hierher ziehen und dort nur noch verwenden. Bearer `wp_con_…`, Long-Poll mit
Timeout `warten + 10` s, Backoff bei Fehlern. Den Cursor je Entität lokal speichern und mit dem Cursor aus `hello`
abgleichen. Idempotenz-Speicher für Aufträge (Schlüssel → Ergebnis) als Datei.

Test: gegen das lokale Backend (`backend/tests/test_connector_v2.py` zeigt die Erwartungen). Ein Vertragstest prüft,
dass die Rust-Typen dieselben Felder haben wie `vertrag.py`, z. B. über eine aus Pydantic erzeugte JSON-Schema-Datei.

Abhängigkeit: nur der Vertrag. Blockiert nichts außerhalb des Repos.

### B. Dienst-Modus (`theorg/dienst`)

- Crate `windows-service`: `praxishub-theorg.exe dienst` läuft unter dem SCM. Dazu kommen die Befehle
  `installieren`, `entfernen`, `pruefen` (Verbindungen testen, ohne Dienst) und `einrichten` (Verbindungscode
  und Datenbankverbindung eintragen; Passwort per verdeckter Eingabe).
- Dienstkonto `NT SERVICE\PraxishubConnector` (virtuelles Konto). Rechte nur auf den eigenen ProgramData-Ordner
  und das THEDEX-Verzeichnis. Das setzt der Installer.
- DPAPI: Mit dem virtuellen Konto gibt es ein eigenes Benutzerprofil, `CryptProtectData` ohne
  `CRYPTPROTECT_LOCAL_MACHINE` funktioniert also. Weil `einrichten` aber unter dem Administrator läuft, muss das
  Verschlüsseln im Dienstkontext passieren. Einfacher ist `CRYPTPROTECT_LOCAL_MACHINE` mit ACL auf die Datei
  (nur Dienstkonto und Administratoren). Entscheidung beim Bau, im Technikblatt steht nur „DPAPI“.
- Protokoll mit `tracing-appender`: Tagesdatei, 14 Tage, ohne Patientendaten. Dafür eine Regel im Code:
  Entitäten nur als `pvs_id` und Anzahl protokollieren, nie Namen oder Geburtsdaten. Ein Test prüft die
  Log-Ausgabe eines Abgleichs mit Testdaten. Bei Fehlern zusätzlich ein Eintrag in der Ereignisanzeige.
- Wartungsfenster: 23–6 Uhr keine Datenbankverbindung, konfigurierbar.
- Updates: Schalter `auto_update` (Standard für IDT: aus) und Zeitfenster 6–7 Uhr. Ein Dienst kann den
  Tauri-Updater nicht nutzen. Daher eigener Ablauf: Manifest vom Feed holen, Minisign-Signatur prüfen (Crate
  `minisign-verify`, gleicher öffentlicher Schlüssel), Authenticode des Installers prüfen und den Installer im
  Silent-Modus starten. Der Installer stoppt den Dienst, ersetzt die Dateien und startet ihn wieder.
- Installer: NSIS oder WiX als Installation für alle Benutzer (`perMachine`). Herausgeber „IntensivKontakt GmbH &
  Co. KG“ (beim Z1-Bundle steht heute „IntensivKontakt GmbH“). Signiert über dieselbe Pipeline wie `build-sign.yml`,
  Tag `theorg-v*`.

Abhängigkeiten: A für einen sinnvollen Testlauf. Cloud-seitig: Kanal `theorg` in `UPDATE_CHANNELS` eintragen,
Download über `api.praxishub.ai` durchreichen statt `browser_download_url` (sonst braucht IDT eine Freigabe für
GitHub). Beides ist eine kleine Änderung im Walletpass-Backend.

### C. Firebird-Leseadapter (`theorg/fbdb`)

- Treiber: `rsfbclient` mit reiner Rust-Implementierung (`rsfbclient-rust`, ohne `fbclient.dll`) oder mit der
  `fbclient.dll`, die auf dem Server mit THEORG ohnehin liegt. Vor dem Bau prüfen, ob die reine Variante die
  Srp-Anmeldung von Firebird 3 und den Zeichensatz von THEORG kann. Dafür reicht die Nachbildung in `theorg/db`
  mit Firebird 2.5, 3.0 und 4.0.
- Verbindung `localhost/3051:<Alias>`, Benutzer `PRAXISHUB_LESEN`. Jede Abfrage in einer Transaktion
  `READ ONLY READ COMMITTED` (in Firebird 4 `READ CONSISTENCY`), keine dauerhaft offene Transaktion. Eine offene
  Transaktion würde die Garbage Collection von THEORG aufhalten.
- Schema-Fingerabdruck: Hash über Tabellen, Spalten und Typen der benötigten Tabellen aus `RDB$RELATION_FIELDS`.
  Er geht im Heartbeat als Komponente `pvs_db` mit. Bei Abweichung schreibt der Adapter nicht mehr, liest aber
  weiter, solange die benötigten Spalten noch da sind.
- Leser je Entität mit Abbildung auf den Vertrag: `behandler`, `patienten`, `termine` (Fenster heute −7 Tage bis
  +12 Wochen), `verordnungen`, `freie_zeiten`. Inkrementell über Änderungsstempel, falls THEORG welche führt
  (Falle 6 in `THEORG-SCHEMA.md`). Sonst liest der Adapter das Fenster ganz, schickt alles, und die Cloud verwirft
  Unverändertes über den Fingerprint.
- **Arbeitszeiten:** Der Vertrag kennt keine Entität für Arbeitszeiten und Abwesenheiten. Die Cloud rechnet freie
  Zeiten entweder aus `freie_zeiten` vom Connector (Weg a in `backend/app/kalender/pvs.py`) oder aus in Praxishub
  gepflegten Arbeitszeiten (Weg b). Für THEORG ist Weg a naheliegend: Der Adapter rechnet Arbeitszeit minus
  Abwesenheit und meldet das als `FreieZeitV2` mit `ersetzt_fenster`. Belegte Termine zieht die Cloud selbst ab.
  Alternative wäre eine neue Entität `arbeitszeiten` im Vertrag. Entscheidung offen.
- Mandanten: Die Pilotpraxis hat 3 Mandanten. Wie THEORG sie trennt (eine Datenbank oder je Mandant eine, Spalte
  in den Bewegungstabellen), zeigt erst die Sichtung. Davon hängt ab, ob eine Installation einen oder drei
  `wp_con_…`-Schlüssel braucht.

Abhängigkeit: **die Kartierung**. Tabellen, Spalten und Status-Codes von THEORG kennen wir erst nach der Sichtung
mit Datenbankzugang, und den gibt es erst nach der Zusage von IDT. Bis dahin lassen sich Verbindung,
Fingerabdruck, Transaktionen, Fensterlogik und Abbildung gegen das Beispielschema der Nachbildung bauen. Die
konkreten SQL-Abfragen kommen in einer Datei je Entität dazu, sobald `THEORG-SCHEMA.md` ausgefüllt ist.

### D. Schreibadapter (später)

Erst nach dem Testtermin an der Entwicklungskopie (`S1-termin-diff.ps1`, Leitfaden §3.7 und Schritt 2) und der
Freigabe durch Praxis und IDT. Benutzer `PRAXISHUB_TERMINE`, eine Transaktion je Auftrag, ID über denselben
Generator wie THEORG. Der Idempotenz-Schlüssel steht in der Terminnotiz oder im lokalen Speicher. Fehlercodes nach
Vertrag §6.2. Vorher prüfen: Wie erfahren andere Arbeitsplätze von neuen Terminen (`POST_EVENT`)? Hält THEORG
einen Zwischenspeicher?

### E. THEDEX im Dienst

Die Bibliothek ist fertig. Im Dienst kommen dazu: der Eingang (`verarbeiten` → `ingest/patienten`,
`mitgliedschaften`, `checkins`) und die Aufträge `patient_anlegen` (`ClientNew`) und `dokument_ablegen`
(`ClientReport`). Abhängigkeit: das THEDEX-Verzeichnis auf dem IDT-Server und die `SenderVerification`.

## 4. Reihenfolge und Aufwand

Schätzung in Arbeitstagen, ohne Erfahrungswerte aus diesem Repo. Nikolas entscheidet den Zeitplan.

| Schritt | Inhalt | Schätzung | wartet auf |
|---|---|--:|---|
| 1 | A: v2-Client mit Tests gegen das lokale Backend | 3–4 | – |
| 2 | B: Dienst, Installer, Logs, Update ohne Tauri; Backend: Kanal `theorg`, Download-Durchreichung | 4–6 | 1 |
| 3 | C ohne Kartierung: Treiberwahl, Verbindung, Fingerabdruck, Fensterlogik gegen die Nachbildung | 3–4 | – |
| 4 | E: THEDEX-Eingang und -Aufträge im Dienst | 2 | 1, 2 |
| 5 | Installation bei IDT, nur lesend (zunächst THEDEX und Verbindungstest), Sichtung mit Datenbank | 1 | 2, Zusage IDT |
| 6 | C mit Kartierung: SQL je Entität, freie Zeiten, Mandanten | 3–5 | 5 |
| 7 | D: Schreibadapter nach Diff-Test und Freigabe | 4–6 | 6, Freigabe |

Die Schritte 1 bis 4 lassen sich vor der Antwort von IDT erledigen. Damit ist das Technikblatt bis auf die
Schreibrechte gedeckt, wenn IDT zusagt.

## 5. Offene Entscheidungen

1. Freie Zeiten: Weg a (Adapter rechnet `freie_zeiten`) oder neue Vertragsentität `arbeitszeiten`?
2. Treiber: reines Rust oder `fbclient.dll`?
3. DPAPI im Dienst: Maschinenschlüssel mit Datei-ACL oder Profil des Dienstkontos?
4. Installer: NSIS (wie Z1) oder MSI (für IDT eventuell gewohnter)?
5. Mandanten: ein Schlüssel je Installation oder je Mandant?
