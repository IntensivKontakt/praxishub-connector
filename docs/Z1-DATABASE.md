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
**⚠ ANTRAGSNUMMER tokenisieren:** Das ZPLAN-Feld enthält hinter der eigentlichen Nummer
mit Leerzeichen eingebettete Zusatzfelder (teils eine zweite Antragsnummer):
`"0300068382606ZE000001811500401 2210 … 1 1"`. `FILEPOOL.FILENAME` trägt nur das **erste
Token** — verifiziert 2026-07-09: Voll-String matcht 1/1355 Pläne, erstes Token 1330/1355
(98 %; Rest hat kein XML). Bei mehreren Dateiversionen (`…01.xml`,`…02.xml` = Nachbesserung)
per `ORDER BY FILENAME DESC` die höchste nehmen.

**EEBZ0-Inhalt (Namespaces `ant:`/`bas:`/`zer:`):** `zer:Behandlungskosten_insgesamt`
(z. B. `1445,37` — deutsches Dezimalformat!), `zer:Honorar_BEMA`/`zer:Honorar_GOZ`,
`zer:Material_und_Laborkosten`, `zer:Leistungsbeschreibung` (je Position, Klartext),
`bas:Zahnarztnummer`. **Kein Patientenanteil im Antrag** — Festzuschuss-Euros stehen erst
in der Antwort (EEBZ1) bzw. `ZEHIT.ZE2`-F3. Behandler des Plans: `ZPLAN.LEBID` (§8c).
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

**Patienten-Matching fürs Rückschreiben (Vorab-Aufnahme):** `z1db/lookup.rs::resolve_patient`
+ `matching.rs`. Kandidaten aus `PAT`(+`ADR` für PLZ) per exaktem Geburtsdatum, plus
Fallback PLZ+Namenspräfix (fängt Geburtsdatum-Tippfehler). Bewertung mit **Damerau-Edit-
Distanz** (Tippfehler/Transposition = 1 Edit) + PLZ-Bonus → `Matched` (sicher, auto),
`Review` (nah dran/mehrdeutig → **manuelle Zuordnung ans Team** via `…/writeback/{id}/
unmatched` mit Kandidaten), `NotFound` (noch nicht in Z1 → zurückstellen). So füllt der
Patient die Anamnese **vorab** aus; sobald er per Kartenstecken in Z1 landet, matcht der
nächste Poll und schreibt automatisch — **kein Neupatient-Anlegen nötig**.

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

### 8a. Geldbeträge — Speicherformat (verifiziert 2026-07-09 an Live-Z1)

**Beträge sind KEINE Dezimalzahl-Strings.** Format = **Währungspräfix `e` (Euro) + Betrag
als ganzzahlige Cent, rechtsbündig, ohne Trennzeichen**, z. B. `"e     6426"` = **64,26 €**,
`"e   225992"` = **2.259,92 €**. (Historisch `d` = D-Mark → daher der Alt-Name `DMBETRAG`.)
Wenige Zeilen haben statt `e` ein Leerzeichen als Präfix — identisch zu parsen. Naives
Zahl-Parsen ergibt `null`; **nur die Ziffern ziehen und `/100`**:
```sql
CAST(NULLIF(REPLACE(SUBSTRING(<feld>, 2, LEN(<feld>)), ' ', ''), '') AS bigint) / 100.0  -- Euro
```
**Stornos sind NICHT negativ** — der Betrag bleibt positiv, `CASH.STORNIERT='1'` markiert die
Umkehr (negieren/ausschließen, kein Minus im Feld suchen).

Quellen nach Verlässlichkeit:
- **Zahlungen → `CASH.BETRAG`** (100 % befüllt) + `STORNIERT`, `ZAHLUNGSWEG`, `CASHDATUM`.
- **Laborkosten → `LBLOCKENTRY.EINZELBETRAG`** (~91 %).
- **Behandlungshonorar hat `BEH` NICHT als Feld.** `DMBETRAG` ist nur bei Privatleistungen
  (GOZ) gefüllt (~33 %), bei BEMA leer (**wird berechnet** aus `LSTNR`×`FAKTOR`×`ANZAHL`
  gegen den Gebührenkatalog `GOART`). Pro-Leistungs-Umsatz kommt aus `BILL`/`FAKT`/`KONTO`.
- Fixkomma gilt Z1-weit: `ANZAHL` ×100 (`"   100"`=1,00), `FAKTOR` ×10000 (`" 35000"`=3,5).

### 8b. Umsatz-Reporting nach Behandler und BEMA/GOZ (verifiziert 2026-07-09)

**Keine einzelne Tabelle hat Euro + Behandler + Gebührenordnung zusammen.** Es sind zwei
Achsen aus verschiedenen Quellen:

- **Behandler-Achse → `BEH.LEBID`** (~100 % befüllt). Name/Kürzel über
  `LEB.LEBID → LEB.PID → PERSONAL.PID` (`PERSONAL.KUERZEL`, Vollname via `PERSONAL.ADRID→ADR`
  oder `GEMATIKVORNAME/NAME`). `LEB.BEZEICHNUNG` ist leer — **nicht** als Name nutzen.
  Behandler-Master: `LEB` (18), Personal: `PERSONAL` (41). `LEB.ZANR` = Zahnarztnummer/LANR.
- **BEMA/GOZ-Achse → `BEH.GOART`**: **`g` = BEMA/GKV**, **`q` = GOZ** (Haupt-Privathonorar),
  `2/3/4/7` = Material/Sonderpositionen (privat, Euro gespeichert). Leer = Befund/kein Honorar.

**Euro je Segment:**
- **GOZ + Privat (`q`,`2`,`3`,`4`,`7`) → direkt aus `BEH.DMBETRAG`** (gespeichert), nach
  `LEBID`×`GOART` summierbar. Keine Berechnung nötig.
- **BEMA (`g`) → NICHT in BEH gespeichert, aber aus dem Katalog berechenbar.** Der
  Gebührenkatalog ist **`GO`** (12,3 k Zeilen, keyed auf `LSTNR`/`KYLSTNR`): Spalte
  **`GEBPKT`** = Bewertungszahl (×100, z. B. `1800`=18,00 Pkt für BEMA „01"), `EINFACHSATZ`
  = GOZ-Einfachsatz (€), `PWLART` → `PUNKTWERTE`. **`BEH.KYLSTNR = GO.KYLSTNR` matcht zu
  100 %** (verifiziert). Formel je Zeile:
  `Euro = GEBPKT/100 × ANZAHL/100 × Punktwert`, Punktwert aus `PUNKTWERTE.PWWEST`
  (Format `e`+Wert, `/10000`; PWLART `a`≈1,33 €/Pkt konservierend) je `GO.PWLART` +
  Kasse-Gruppe (`PWGROUP`) + gültig ab `ABDATUM`. **Wichtig:** `GO`-Gültigkeit über
  `ABDATUM<=Datum AND (BISDATUM IS NULL OR BISDATUM=''/>=Datum)` — `BISDATUM` ist NULL,
  nicht `''`. `GO.GOART` ≠ `BEH.GOART` (GO nutzt u. a. `g`=zahnärztl., `h`=EBM/GOÄ) —
  **nicht** danach filtern, über `KYLSTNR` joinen.
- **Reconciliation-Warnung:** Die reine `GOART='g'`-Berechnung ergibt nur **konservierenden**
  BEMA-Umsatz. `FAKT.ZHON` (KTRAEGER=1) ist der **gesamte** GKV-Honorartopf (KCH **+** ZE +
  PAR + KFO + KBR + Zuschläge) und zeitversetzt (Rechnungs- ≠ Behandlungsdatum, Quartale).
  Beispiel 2026: berechnet 310 k (nur `g`) vs. fakturiert 584 k (alle Sparten). Für
  „konservierender BEMA nach Behandler" ist die Katalog-Methode korrekt; für den **gesamten**
  GKV-Honorarumsatz weiter `FAKT` nehmen und per BEMA-Punktanteil auf Behandler aufschlüsseln.

### 8c. GKV-Sparten pro Behandler — Join-Keys + Sammelkonto-Realität (verifiziert 2026-07-09)

`FAKT.ZHON` (KTRAEGER=1) zerlegt sich exakt nach `RART`: `5010`=**KCH** (~398k, kein Labor),
`5050`=**PAR** (~95k), `5060`/`6020`=**ZE** (~91k, mit Labor). Behandler-Zuordnung je Sparte:

- **KCH** → über `BEH.LEBID` (Katalog-Methode §8b). Nicht plan-basiert.
- **PAR** → `FAKT.PATNR+LFDPATBILL = PARHIT.PATNR+LFDPATBILL` → `PARHIT`⋈`ZPLAN(PATNR,LFDPLAN)`
  → `ZPLAN.LEBID`. **`PARHIT.GLFDFAKT` ist leer — nicht als Key benutzen.**
- **ZE** → `FAKT.PATNR+LFDFAKT = BILL.PATNR+GLFDFAKT` → `BILL.LFDHPLAN = ZPLAN.LFDPLAN`
  → `ZPLAN.LEBID`. (ZEHIT/PARHIT selbst haben **kein** `LEBID`; der Behandler kommt aus `ZPLAN`.)
- **`ZPLAN.LEBID`** ist die universelle Behandler-Achse für alle Pläne (ZE/PAR/KBR), 100 % befüllt.

**★ Behandler IMMER fallseitig zuordnen, nicht rechnungsseitig.** Ein großer Teil des
GKV-ZE/PAR-Honorars wird über **Sammelrechnungen** gebündelt — Pseudo-`PATNR` wie
**`0kz`/`0kb`** (nicht-numerisch, kein eigener `ZPLAN`). Das heißt **nicht**, dass diese
Honorare keinem Behandler zuordenbar sind: **jeder gebündelte Fall ist eine `ZEHIT`/`PARHIT`-
Zeile mit echtem Patienten und über `ZPLAN.LEBID` einem konkreten Behandler** (verifiziert
143/143 ZE, 194/194 PAR). Nur die *Sammelrechnung selbst* (`FAKT`) lässt sich nicht splitten.
→ **Für „Umsatz nach Behandler" auf der Fall-/Planebene aggregieren (`ZEHIT`/`PARHIT` →
`ZPLAN.LEBID`), nicht auf der Rechnungsebene.** `KFOHIT`=0 (Praxis macht kein KFO).

**⚠ ZE-Honorarbetrag NICHT rekonstruieren.** Drei Rechenwege (`(VB+NB)×PWZE`=14k;
`ZEHITLST⋈GO×PWZE`=556k; `ZEHITLST.LST2`=809k) wichen 2026 alle stark von den fakturierten
~92k ab — das ZE-Modell (BEMA-ZE + GOZ-Verblendung + Festzuschuss + Faktoren, gemischt je
Fall) ist nicht verlässlich nachrechenbar. **Autoritativen Betrag verwenden** (`FAKT.ZHON`)
und über den Fall auf `ZPLAN.LEBID` zuordnen. (Gleiche Lehre wie beim `e`+Cent-Format in
§8a: gespeicherten/fakturierten Wert lesen, nicht rekonstruieren.)

**★ Sammelrechnung→Fälle-Verkettung (verifiziert, reconciled):**
1. Sammelkonten = Pseudo-`PATNR` **`0kz`** (ME-ZE), **`0kp`** (ME-PAR), **`0kb`** (ME-KBR),
   monatliche DTA-Läufe. Periode aus `FAKT.BESCHREIBUNG` = `ME-XX n/MM.JJJJ` (mehrere Läufe
   je Monat über `RIGHT(BESCHREIBUNG,7)` aggregieren). Achtung: 0kb läuft unter RART 5060,
   ist aber KBR, nicht ZE.
2. Monats-Kohorte = Fälle mit `DTADATUM` derselben Periode (`ZEHIT`/`PARHIT`/`KBRHIT`;
   `DTADATUM<>'00000000'`; `ABRSTATUS='4'`=im DTA abgerechnet).
3. Monatssumme `FAKT.ZHON` **pro-rata** auf die Kohorte verteilen — Gewicht ZE = **`ZE2`-Feld
   F4** (s. u.; echte Kassenzahlung je Fall), PAR/KBR = `SUMTOTAL`.
   **`ZEHIT.ZE2`-Blob-Layout (entschlüsselt):** 80 Zeichen = 5 Geldfelder à 10 (`e`+Cent,
   §8a-Format) an Pos. **1, 11, 21, 61, 71** + 30 Zeichen Text (Ort) an Pos. 31–60.
   **F4 (Pos. 61–70) = Gesamt-Kassenzahlung des Falls inkl. Laboranteil** (Monatssummen
   ≈99 % von `FAKT ZHON+ELAB+FLAB`), F3 (Pos. 21) = Festzuschuss-Brutto. Eine separate
   Honorar-Komponente je Fall existiert NICHT als Feld → F4 als Verteil-Gewicht nutzen:
   `TRY_CAST(NULLIF(REPLACE(REPLACE(SUBSTRING(ZE2,61,10),'e',''),' ',''),'') AS bigint)`.
4. Behandler je Fall = `ZPLAN.LEBID` (100 %), Name via `LEB.PID→PERSONAL.KUERZEL`.
5. Einzelrechnungen (echte `PATNR`): PAR `FAKT(5050)⋈PARHIT` über `PATNR+LFDPATBILL`;
   ZE `FAKT⋈BILL(PATNR,GLFDFAKT=LFDFAKT)⋈ZPLAN(LFDPLAN=BILL.LFDHPLAN)`.

Reconciliation 2026: PAR exakt (94 978 €), KBR exakt (28 697 €), ZE-Sammel exakt
(33 257,32 € mit F4-Gewichten). Ergebnis pro Behandler (PAR+ZE+KBR): lwt 60k, st 49k,
swt 45k, mm 32k. Pro-rata bleibt eine **Näherung innerhalb des Monats** (die
Honorar-Komponente je Fall ist nirgends gespeichert), aber mit F4 = echter Kassenzahlung
je Fall gewichtet; Summen pro Monat/Sparte sind per Konstruktion exakt.

**Rechnungstabellen (Rechnungswahrheit, ohne Behandler-Granularität):**
- **`FAKT`** (64 k) = Rechnung: `BETRAG` gesamt, `ZHON` (Zahnarzthonorar), `ELAB`/`FLAB`
  (Eigen-/Fremdlabor) + `…UST`, K-Varianten = Kassenanteil; `KTRAEGER` (`1`=GKV, `z`/`a`=privat),
  `RART` (Rechnungsart: 5xxx GKV, 7xxx privat), `STORNIERT`, `FAKTDATUM`. Beträge im `e`+Cent-Format.
- **`BILL`** (97 k) = Abrechnungsfall: `BETRAG`, Behandlungszeitraum (`VONBEHDATUM`/`BISBEHDATUM`),
  `LFDHPLAN`, Verweise auf die G/P/S-Faktura (`GLFDFAKT`/`PLFDFAKT`/`SLFDFAKT`). Kein `LEBID`/`GOART`.
- **`CASH.BETRAG`** = tatsächliche Zahlungseingänge (s. 8a).

**Reporting-Empfehlung:** BEH als Leistungs-/Behandler-Achse (GOZ-Euro direkt, BEMA top-down
aus FAKT), Summen gegen `FAKT` (fakturiert) und `CASH` (bezahlt) plausibilisieren. Erprobte
Aggregation:
```sql
SELECT p.KUERZEL, b.GOART, COUNT(*) AS leistungen,
       SUM(CAST(NULLIF(REPLACE(REPLACE(b.DMBETRAG,'e',''),' ',''),'') AS bigint))/100.0 AS goz_euro
FROM BEH b
LEFT JOIN LEB l      ON LTRIM(RTRIM(l.LEBID)) = LTRIM(RTRIM(b.LEBID))
LEFT JOIN PERSONAL p ON LTRIM(RTRIM(p.PID))   = LTRIM(RTRIM(l.PID))
WHERE b.DATUM >= '20260101' AND LTRIM(RTRIM(b.GOART)) <> ''
GROUP BY p.KUERZEL, b.GOART;   -- g-Zeilen: goz_euro = NULL (BEMA, s. o.)
```
> Die genaue Legende der Privat-GOART-Codes (`q`/`2`/`3`/`4`/`7`) ist aus Füllmuster +
> Größenordnung abgeleitet; `g`=BEMA vs. Rest=privat ist sicher. Exakte Code-Bedeutung bei
> Bedarf einmal an einem bekannten Fall gegenprüfen.

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
`writeback_contact / _address / _cave / _anamnese / _new_patient`
+ **`z1_hkp_lookback_months`** (Default 24): abgeschlossene/abgelehnte HKP-Fälle nur
bis so weit zurück melden (Effizienz); offene/abgelaufene IMMER. `0`=unbegrenzt. Das
**FE** filtert die Anzeige feiner (Default z. B. 6 Monate, einstellbar).
Tauri-Commands: `test_z1db_connection`, `bootstrap_z1_readonly`.

**Robustheit:** Verbindungs-Timeout (12 s) + Poll-Zyklus-Timeout (120 s) verhindern,
dass ein hängender Query den Dienst/Stop blockiert. Change-Detection per Fingerprint,
StatusStore einmal pro Zyklus persistiert; Writeback idempotent
(`applied_writebacks.json` + „CAVE bereits vorhanden"-Check).

Cloud-Verdrahtung umgesetzt: `hkp.rs` (HKP-Poller EBZ→Cloud, `report_hkp_status`),
`writeback.rs::spawn` (Cloud→Z1, mit Idempotenz-Store), `lookup.rs::resolve_patnr`
(Name+Geb→PATNR). Beide Schleifen im Tauri-Lebenszyklus verdrahtet.

**HKP-Lifecycle (voller Status, nicht nur Entscheidung):** `hkp.rs` leitet je Plan aus
allen `EBZ`-Zeilen + `ZPLAN`/`ZEHIT` den Status ab und meldet **Statuswechsel**:
`erstellt` (inkl. signiert) → `versendet` → `rueckfrage` (DOKART=4 der Kasse, Aktion
nötig) → `genehmigt`/`abgelehnt` (DOKART=3 ZUGESTELLT) → `eingegliedert`
(ZEHIT.EINGLIEDERUNGSDATUM) → `abgerechnet` (**nur** ZPLAN.KZVABRDATUM;
KZVEINREICHDATUM ist die Einreichung, schon bei Genehmigung gesetzt — NICHT Abrechnung).

**Fall-Gruppierung (eine Kachel pro Fall, nicht pro Plan):** Fall = `(PATNR, LFDBEFUND)`.
Ein Fall bündelt den **GAV-Plan** (`PLANART='3'`, Kasse, EBZ-getrackt) + die **AAV-Variante**
(`PLANART='a'`, private Alternative, verknüpft via `LFDAPLAN`, kein EBZ-Status → `privat`).
Rückfrage-Nachreichungen = **derselbe LFDPLAN** (mehrere EBZ-Zeilen, im Verlauf). Echte
Um-Planung = neuer LFDPLAN+LFDBEFUND, **kein** Z1-Vorgänger-Link (`UPTALTPLAN`=IK-Nr!). Der
Poller meldet **fall-zentriert** (`HkpCaseReport`): Fall-Status vom führenden GAV-Plan +
Meilenstein-Daten + Voll-HKP-XML + `plans[]` (alle Pläne des Falls mit EBZ-`submissions[]`
= Antrag/Antwort/Rückfrage/Nachreichung) fürs Detail-Drawer.

**★ `abgelaufen` (Werthebel):** genehmigt, aber nicht eingegliedert und entweder in Z1
deaktiviert (`PLANSTATUS=6`/`DEAKTIVIERTDATUM`) **oder** über die Gültigkeit
(Genehmigung + 6 Monate) hinaus. Praxis-Realität (verifiziert 2026-07-08, eHKP): 509
eingegliedert, 129 deaktiviert, 239 genehmigt-offen — davon **157 über 6 Monate alt,
nicht deaktiviert = „still verloren"**. Report liefert `valid_until` (Genehmigung+6M) →
Praxishub bildet „Tage bis Ablauf" und „genehmigt & nicht terminiert" (Terminierung
kommt Praxishub-seitig; Z1-Terminmodul `ETSSTERMIN` leer → Doctolib). Report trägt
Meilenstein-Daten + **Voll-HKP-EEBZ0-XML** (Detail-Drawer; Rendern per KZBV-XSLT =
„PDF-Ansicht", ein separates HKP-PDF gibt es in Z1 NICHT).

**Noch offen:** Backend-Routen unter `/connector/z1/*` (hkp-status, writeback/pending
+ ack); UI der Toggles; Neupatient-Anlage (NUMBERPOOL + Karten-Match-Test); Build/Test
auf der Dev-Maschine (kein `cargo` am PVS — ein paar `tiberius`-API-Details verifizieren).

## Sicherheit

- **Keine Passwörter** in dieses Repo. Connector-Secrets liegen DPAPI-geschützt in der
  Config (an den Windows-Benutzer gebunden).
- Der Connector nutzt ausschließlich `praxishub_ro` (`db_datareader`) — **kein** Schreiben
  über die DB. Admin-Zugangsdaten werden nur transient zum einmaligen Anlegen des
  Read-only-Logins verwendet und **nicht** gespeichert.
