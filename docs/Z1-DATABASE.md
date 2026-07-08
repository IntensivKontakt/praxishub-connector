# Z1-Datenbank — Referenz für Praxishub

> **Status:** verifiziert am 2026-07-07 an einem Live-Z1 (Praxis ZMM), Z1 v2.96.
> Diese Datei ist die **kanonische Referenz** für den Zugriff des Connectors auf die
> Z1-Datenbank. Sie enthält **keine Passwörter** (die liegen in der Connector-Config,
> DPAPI-geschützt, bzw. werden bei der Einrichtung out-of-band gesetzt).

Die Z1-SQL-Datenbank ist die **zentrale Infrastruktur** für Praxishub: HKP-Status +
-Volltext, Patienten-Stammdaten, Versicherung und ID-Matching kommen von hier — **live,
read-only, ohne KIM/TI**. Diese Datei beschreibt, was wo liegt und wie man es liest.

## 1. Zugang

| | |
|---|---|
| Server / Instanz | `srv-fs\z1` (SQL Server, named instance; erreichbar per SQL Browser) |
| Hauptdatenbank | `Z1` |
| Weitere DBs | `CGMArchive` (PraxisArchiv, **eigene Auth — für z1/RO gesperrt**), `z1trigger` (Replikations-Journal), System-DBs |
| Auth | **SQL-Auth** (kein Integrated). App-Login = `z1` (Lese+Schreib), Admin = `sa`. |
| Connector-Login | **`praxishub_ro`** — dediziert, `db_datareader` auf `Z1` (nur SELECT). |
| ODBC | System-DSN `Z1` (Driver 18), `TrustServerCertificate=Yes`. Der Connector verbindet direkt per TDS (kein ODBC nötig). |

**Dedizierten Read-only-Login anlegen** (einmalig, als `sa`; Passwort selbst wählen):
```sql
CREATE LOGIN [praxishub_ro] WITH PASSWORD = N'<PASSWORT>', CHECK_POLICY = OFF, DEFAULT_DATABASE = [Z1];
USE [Z1];
CREATE USER [praxishub_ro] FOR LOGIN [praxishub_ro];
ALTER ROLE [db_datareader] ADD MEMBER [praxishub_ro];   -- nur lesen, kein Schreiben
```
Widerruf: `DROP USER praxishub_ro` / `DROP LOGIN praxishub_ro`.

## 2. Datenbank-Konventionen (gelten für alle Tabellen)

- **136 Tabellen**, praktisch alle Spalten `varchar` (auch Zahlen/Datum).
- **Datum** = String `JJJJMMTT` (8 Zeichen), Uhrzeit teils `HH:MM:SS`.
- **`PATNR`** = 10 Zeichen, **rechtsbündig mit Leerzeichen aufgefüllt** → beim Vergleich
  immer `LTRIM(RTRIM(PATNR))`.
- **`RINFO`** (34 Zeichen) auf jeder Tabelle = Sync-/Concurrency-Stempel, **von der App
  gesetzt** (kein DB-Default). Format: `JJJJMMTTHHMMSSmmm` + Arbeitsplatz-Kürzel + Zähler
  + Flags, z. B. `20260707090555617  2aks  2 442670`.
- **3 Trigger pro Tabelle** (Insert/Update/Delete) schreiben jede Änderung als Journal-
  Zeile nach `Z1TRIGGER.Z1.Z1TRIGGER296` → **CGM-Replikations-/Sync-Mechanismus**.
- **`NUMBERPOOL`** = zentrale Nummernvergabe (eine Zeile mit Hochzählern `PATNR`, `ADRID`,
  `PID`, …). Neue Datensätze ziehen ihre ID hier (kein IDENTITY).

## 3. HKP-/EBZ-Tracking (der Kern für Praxishub)

### 3a. Status-Feed — Tabelle `EBZ` (elektronische Übertragung + Entscheidung)
Key `PATNR + LFDPLAN + LFDNR`. Mehrere Zeilen je Plan nach `DOKART`:
- **`DOKART='1'`** = Antrag: `ERSTELLDATUM`, `SIGNATURDATUM`, `VERSANDDATUM`.
- **`DOKART='3'`** = Kassen-Antwort: `ERHALTDATUM` = Entscheidungsdatum,
  **`ZUGESTELLT` = Entscheidung: `1`=genehmigt, `0`=abgelehnt**.

```sql
-- Neue Entscheidungen seit letztem Poll:
SELECT PATNR, LFDPLAN, LFDNR, ERHALTDATUM, ZUGESTELLT
FROM   EBZ
WHERE  DOKART = '3' AND ERHALTDATUM > @last_seen;   -- je Plan neueste Antwort nehmen
```
Abgeleiteter UI-Status (= „eDokumentenverwaltung"/„Planverwaltung"-Spalte *Status*):
neueste `DOKART='3'`-Antwort da → `ZUGESTELLT=1` genehmigt / `=0` abgelehnt; sonst
`VERSANDDATUM` gesetzt → versendet; sonst `SIGNATURDATUM` gesetzt → signiert.

### 3b. Voll-HKP-Inhalt — Tabelle `FILEPOOL` (Blob-Store)
`FILENAME` + `FILEDATA varbinary(max)` + `FILEDIR` (`data\EBZ\<PatNr>\ZE|PA\`).
Enthält die kompletten EBZ-Dokumente **live** (der frühere Datei-Store `…\backupdata\ebz`
ist eingefroren; die DB läuft weiter). Verknüpfung über die Antragsnummer:
```sql
-- Antragsnummer des Plans:
SELECT ANTRAGSNUMMER FROM ZPLAN WHERE PATNR=@patnr AND LFDPLAN=@lfdplan;
-- Voll-HKP als offizielles GKV-XML (EEBZ0), + Signatur .p7s, + Antwort EEBZ1:
SELECT FILENAME, FILEDATA FROM FILEPOOL WHERE FILENAME LIKE 'EEBZ0_' + @antragsnummer + '%.xml';
```
Das **`EEBZ0_*.xml`** ist der vollständige HKP: Zahnbefunde je Zahn, Regelversorgung,
Befunde für Festzuschüsse + Zuschusshöhe, `Leistung_BEMA`/`Leistung_GOZ` +
`Gebuehrennummer_*` + `Honorar_*`, Material-/Laborkosten, `Behandlungskosten_insgesamt`,
Versicherter/Kasse/Zahnarzt. Rendern per offizieller KZBV-XSLT (oder eines der PDFs im
FILEPOOL). `EEBZ1_*.xml` = Genehmigungs-/Ablehnungs-Antwort.

### 3c. Plan-Stammdaten — Tabelle `ZPLAN` (Planverwaltung, inkl. Privatpläne)
Key `PATNR + LFDPLAN`. Wichtige Felder: `PLANART`, `KASSENPLAN`, `ANTRAGSNUMMER`,
`MITTEILUNGSNUMMER`, `PLANSTATUS`, `PLANUNGSDATUM`, `DRUCKDATUM`, `KZVEINREICHDATUM`,
`GENEHMIGUNGSDATUM`, `DEAKTIVIERTDATUM`.
**PLANART-Codes:** `3`=eHKP/ZE GAV (Antragsnr enthält „ZE"), `a`=eHKP AAV/privat
(nicht eingereicht), `4`=ePAR („PA"), `7`=eKBR/KGL („KG"), `2`=alt-ZE Kasse.
Andere Plan-Typen mit eigenen Tabellen: `PARHIT`/`PARHITLST` (PAR), `KFOHIT` (KFO),
`KBRHIT`/`KBRHITLST` (KBR/KGL), `ZEHIT`/`ZEHITLST` (ZE-Historie).

## 4. Patienten-ID & Matching — Tabelle `PAT` (18k Patienten, 66 Sp.)

- **PK = `PATNR`** = maßgebliche Z1-Patienten-ID **und** die VDDS-PATID fürs Ablegen.
- Index `K2PAT(KYPATNAME, KYPATVORNAME, PATNR)` → **Name+Vorname+Geb → PATNR** ist eine
  schnelle indizierte Query. `KY*` = normalisierte Suchschlüssel, `PATNAME/PATVORNAME` =
  Klartext, `GEBDATUM` = Geburtsdatum.
- `EXTERNID` = frei belegbare externe ID (für dauerhafte Praxishub↔Z1-Verankerung),
  `VXPATIENTUID` = CGM-Cloud-UID, `LPATNR` = Karteikartennummer.
- Flags: `VERSTORBENAM`, `GESPERRT`. Adress-FKs `ADRIDP/R/A/K/…` → `ADR`.

> **Ablösung Weg A:** Der bisherige Name+Geb→PATID-Lookup über die PraxisArchiv-COM-DB
> kann durch eine direkte `PAT`-Query ersetzt werden (robuster, kein PowerShell-Sidecar).

## 5. Stammdaten-Anreicherung (vollständiger Patientendatensatz)

Join `PAT` + folgende Tabellen:
- **`ADR`** (über `PAT.ADRIDP` u. a.): `TITEL, VORNAME, NAME, STR, PLZ, ORT, LANDKUERZEL,
  SEX, BRIEFANREDE, TELEFON1..7, SECUREMAIL, GEBDATUM, GEBORT, BERUF`, Bankdaten (IBAN…).
- **`VDESC`** (Key `PATNR + LFDPATVD`; aktuellste Periode = neueste `VDABDATUM`,
  `INVALID`-Flag beachten): Versicherten-/eGK-Stammdaten — `VERSICHERTENNR`, `VKNR`,
  `KVKKASNAME`, `Z1KASKUERZEL`, `KSART`, Versichertenart (`MFRDIG/RSA/WSO`),
  `EINLESEDATUM`, `GUELTIGBISDATUM`, `EGKVSD` (roher eGK-VSD-Blob), `GEBUEHRENBEFREITBIS`.
- **`Z1KASSEN`** (über `Z1KASKUERZEL`): Kassenname `BKVKASNAME`, `VKNR`, `KASSENART`,
  **`EBZIK`** (IK — matcht `ik_krankenkasse` im HKP-XML).
- **`PATINFO`** (Key `PATNR + DATUM + …`): Patienten-Zeitachse mit Anamnese-/Fragebogen-/
  Terminverweisen (`LFDANAMNESE`, `LFDFRAGEBOGEN`, `TERMIN`, `STATUS`).

## 6. Anamnese, Formulare, Einwilligungen, Dokumente

- `FRAGEBOGEN` / `FRAGEBOGENENTRY` = Anamnese-Fragebogen**vorlagen** (FRAGETEXT,
  ANTWORTART, CONTROL, PFLICHT). Ausgefüllte Bögen je Patient via `PATINFO`.
- `EINWILLIGUNG` = Einwilligungen (`EINWILLIGUNGART`, `UNTERSCHRIFTDATUM/-ART`,
  `WIDERRUFDATUM`, `LFDARCHIV`, `DOKUMENTKEY`) → verlinkt auf `ARCHIV`.
- **`ARCHIV`** = Dokument-Index je Patient: `PATNR`, `LFDARCHIV`, `OBJEKTART`,
  `OBJEKTDATUM`, `OBJEKTBESCHREIBUNG`, **`BVS`, `MMOID`** (VDDS-media-Kennungen),
  `EPAUNIQUEID`. Der unterstützte VDDS/BVS-Ablageweg registriert Dokumente hier.
- `KOMLEMAIL` = KIM-Mails in Z1 (Spalte `DIENSTKENNUNG`) — die rohe ANW-Nachricht läge
  also sogar hier; für das Status-Tracking aber nicht nötig.

## 7. Schreibzugriff / Rückschreiben von Anamnese-Daten in Z1

**Frage:** Können die bei der digitalen Aufnahme gesammelten Stamm- und
Behandlungsdaten in Z1 geschrieben werden? — **Technisch ja; der saubere Weg hängt vom
Datentyp ab.**

**A. Dokumente (Anamnese-PDF, Einwilligung) → VDDS-media (sanktioniert, bereits gebaut).**
Der Connector legt das unterschriebene PDF über VDDS-media/BVS in die Akte; Z1 registriert
es selbst in `ARCHIV`. Für viele Praxen ist „Anamnese-PDF in der Akte" bereits das Ziel.
**Das ist der empfohlene Schreibweg.**

**B. Strukturierte Felder in EXISTIERENDE Datensätze schreiben (z. B. Kontaktdaten in
`ADR`) → verifiziert machbar & umkehrbar.** Am 2026-07-07 getestet (Patient 16006,
`ADR`-Felder `TELEFON1`+`SECUREMAIL`, beide vorher leer): 1 Zeile aktualisiert, korrekt
zurückgelesen, Replikations-Journal erfasste die Änderung (Arbeitsplatz BUERO2).
Erkenntnisse:
- Die 3 Trigger sind **reines Change-Data-Capture** (schreiben nur Alt→Neu nach
  `Z1TRIGGER.Z1.Z1TRIGGER296`). Sie **erzwingen `RINFO` NICHT** und lehnen nichts ab.
- **`RINFO` trotzdem app-treu neu setzen:** 17-stelliger Zeitstempel `yyyyMMddHHmmssfff`
  + unveränderter Rest des bisherigen RINFO (Arbeitsplatz+Zähler) → Concurrency bleibt sauber.
- Journal-`ARBEITSPLATZ` (=`HOST_NAME()`) füllt SQL Server per Default; nur `KONTEXT`
  (PID/PROGID) bleibt leer, weil außerhalb einer Z1-App-Sitzung geschrieben — kosmetisch.
- **Pflicht-Vorgehen:** Vorher-Wert + RINFO sichern (Restore); nur betroffene Felder
  ändern; Transaktion + `@@ROWCOUNT=1`-Assertion; Datensatz auf Nicht-Freigabe prüfen
  (`ADRID` nicht von mehreren Patienten genutzt).

**C. Neuen Datensatz anlegen (Neupatient) → deutlich riskanter, noch offen.** Braucht
atomare ID-Vergabe aus **`NUMBERPOOL`** (PATNR/ADRID/…) + Mehr-Tabellen-Konsistenz
(PAT+ADR+ggf. VDESC+PATINFO). Noch nicht getestet.

**Allergien/medizinische Anamnese — Speicherort noch NICHT sicher lokalisiert:** keine
eigene Allergie-Tabelle; `PAT.ANAMNESE` ist nur ein kurzes Freitext-Notizfeld (in der
Test-Praxis mit einem Verrechnungsvermerk belegt) — dort NICHT blind reinschreiben.
Vor einem Allergie-Write erst das korrekte Ziel klären (Kandidaten: `PATINFO` mit
Anamnese-ART, `FREITEXT`, oder das Anamnese/Risiken-Modul).

**Allgemein:** unsupported, kann bei Z1-Updates brechen (Schema/Trigger/Format
undokumentiert). Nur mit Einstellungs-Toggle + Test gegen Backup-DB ausrollen.

**C. eGK-Vorbehalt:** Versicherungs-Stammdaten (Name/Adresse/Kasse) sind in DE **autoritativ
die eGK-Kartendaten** (`VDESC`/VSD), nicht Patienten-Selbsteingabe. Diese Felder sollten
**nicht** aus dem Aufnahmeformular überschrieben werden (Abrechnungsrisiko) — sie füllen
sich beim Kartenstecken. Genuin additiv aus der Aufnahme: **Kontaktdaten (Tel./E-Mail)**
und **medizinische Anamnese (Allergien, Medikamente, Vorerkrankungen)**.

**Empfehlung:** DB = **Lese**-Weg (Status, Voll-HKP, Stammdaten, Matching). **Schreiben**
über VDDS-media (Dokumente). Strukturiertes Rückschreiben nur als bewusstes, separat
freigegebenes Feature — vorher gegen eine **Test-/Backup-DB** validieren und prüfen, ob
CGM eine sanktionierte Patienten-Import-Schnittstelle anbietet (`SCHNITTSTELLEN`-Tabelle
ist hier leer → aktuell keine GDT/BDT-Schnittstelle lizenziert/konfiguriert).

## 8. Weitere für Praxishub nutzbare Daten (Connector einmal breit bauen)

| Zweck | Tabellen |
|---|---|
| Abrechnungs-/Zahlstatus | `BILL`, `FAKT`, `KONTO`, `CASH` |
| Leistungshistorie (alle erbrachten Leistungen) | `BEH` (1,46 Mio.) |
| Recall | `HISTRECALL` + PAT-Recallfelder |
| eGK-/Kartenstatus | `VDESC.EINLESEDATUM`, `PRUEFNACHWEIS` |
| ePA-Dokumente | `EPADOCUMENT` |

## Connector-Anbindung (Code)

Umgesetzt im Core-Modul **`core/src/z1db/`** (Treiber: `tiberius`):
- `client.rs` — Verbindung (Named Instance via SQL Browser), `RINFO`-Erzeugung,
  Feld-Padding, Query-/Exec-Helfer.
- `writeback.rs` — `apply_writeback()` schreibt Kontakt/Adresse (`UPDATE ADR`),
  CAVE (additiv `PAT.ANAMNESE`) und Anamnese (`INSERT PATINFO` ART=1) je nach Toggle.
- `bootstrap.rs` — `create_readonly_login()` legt aus temporären Admin-Daten den
  `praxishub_ro`-Login an (Admin-Daten werden nicht gespeichert).

Config (`core/src/config.rs`, DPAPI-geschützt): `z1_db_server/database/user/password`
(Read-only) + `z1_db_write_user/password` (schreibfähig) + Toggles
`writeback_contact / _address / _cave / _anamnese / _new_patient`.
Tauri-Commands: `test_z1db_connection`, `bootstrap_z1_readonly`.

Cloud-Verdrahtung umgesetzt: `hkp.rs` (HKP-Poller EBZ→Cloud, `report_hkp_status`),
`writeback.rs::spawn` (Cloud→Z1, mit Idempotenz-Store), `lookup.rs::resolve_patnr`
(Name+Geb→PATNR). Beide Schleifen im Tauri-Lebenszyklus verdrahtet.

**HKP-Lifecycle (voller Status, nicht nur Entscheidung):** `hkp.rs` leitet je Plan aus
allen `EBZ`-Zeilen + `ZPLAN`/`ZEHIT` den Status ab und meldet **Statuswechsel**:
`erstellt` (inkl. signiert) → `versendet` → `rueckfrage` (DOKART=4 der Kasse, Aktion
nötig) → `genehmigt`/`abgelehnt` (DOKART=3 ZUGESTELLT) → `eingegliedert`
(ZEHIT.EINGLIEDERUNGSDATUM) → `abgerechnet` (ZPLAN.KZVEINREICH/KZVABRDATUM). Report
trägt Meilenstein-Daten + **Voll-HKP-EEBZ0-XML** (fürs Detail-Drawer; Rendern per
KZBV-XSLT = „PDF-Ansicht", ein separates HKP-PDF gibt es in Z1 NICHT). **Terminierungs-
Status** kommt Praxishub-seitig (Z1-Terminmodul `ETSSTERMIN` hier leer → Doctolib).

**Noch offen:** Backend-Routen unter `/connector/z1/*` (hkp-status, writeback/pending
+ ack); UI der Toggles; Neupatient-Anlage (NUMBERPOOL + Karten-Match-Test); Build/Test
auf der Dev-Maschine (kein `cargo` am PVS — ein paar `tiberius`-API-Details verifizieren).

## Sicherheit

- **Keine Passwörter** in dieses Repo. Connector-Secrets liegen DPAPI-geschützt in der
  Config (an den Windows-Benutzer gebunden).
- Der Connector nutzt ausschließlich `praxishub_ro` (`db_datareader`) — **kein** Schreiben
  über die DB. Admin-Zugangsdaten werden nur transient zum einmaligen Anlegen des
  Read-only-Logins verwendet und **nicht** gespeichert.
