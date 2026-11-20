# praxishub-thedex

THEDEX-Dateischnittstelle von THEORG (SOVDWAER) für den THEORG-Connector. Reine Rust-Bibliothek
ohne Tauri und ohne Netzwerk; Grundlage ist die THEDEX-Spezifikation 2.1 vom 15.01.2015.
Der Z1-Connector (`core`, `src-tauri`) ist davon unabhängig.

```
cargo test   -p praxishub-thedex
cargo clippy -p praxishub-thedex --all-targets -- -D warnings
cargo run    -p praxishub-thedex --bin thedex-probe -- <THEDEX-Ordner> [Unterordner]
```

## Was THEDEX liefert und was nicht

| Über THEDEX | Nachricht / Abschnitt | Richtung |
|---|---|---|
| Personenstammdaten (Name, Geburtsdatum, Geschlecht, Adresse, Telefon, E-Mail, Kostenträger-Kennung) | `ClientNew`, `ClientChange`, `ClientInfo`, `[Client]`/`[ClientNewData]` | THEORG → Praxishub |
| Löschung einer Person | `ClientDelete` | THEORG → Praxishub |
| Mitgliedsvertrag (Beginn, Ende, Kündigung, Ruhezeit, Bezeichnung) | `[ContractData]` | THEORG → Praxishub |
| Chip/Check-in-Medium | `CCheckInID` | THEORG → Praxishub |
| Check-in/Check-out | `ClientCheckInOut`, `[CheckInOutData]` | THEORG → Praxishub |
| Neuanlage einer Person | `ClientNew` | Praxishub → THEORG |
| Dokument in die elektronische Akte (PDF, JPG, TXT, RTF, DOC, XLS, HTM, BMP, PCX) | `ClientReport` + Anhangsdatei | Praxishub → THEORG |

Nicht über THEDEX: Termine, Verordnungen, Behandler, Leistungen, Zahlungen und offene Posten.
Dafür braucht der Connector später den Lesezugriff auf die Firebird-Datenbank von THEORG.

## Ordner und Provider

```
<THEDEX>\THEDEX.INI          [THEDEX-Providers]  eine Zeile je Provider: NAME=Beschreibung
<THEDEX>\THEORG\             Eingang von THEORG – hier legt Praxishub ab
<THEDEX>\PRAXHUB\            Eingang von Praxishub – hier legt THEORG ab
<THEDEX>\PRAXHUB\FEHLER\     nicht lesbare Dateien (mit <Name>.TXT = Grund)
```

- Providername = Verzeichnisname, höchstens 8 Zeichen. Vorschlag `PRAXHUB` (konfigurierbar).
- `ThedexOrdner::einrichten` legt das eigene Verzeichnis an und setzt genau die eigene Zeile in
  `[THEDEX-Providers]`. Die Datei wird byteweise bearbeitet: andere Zeilen, Abschnitte,
  Zeilenenden und Zeichensatz bleiben unverändert; doppelte eigene Zeilen werden entfernt. Fehlt
  `THEDEX.INI`, gibt es einen Fehler statt einer neuen Datei (vermutlich falscher Pfad).
- Eingang: `eingang_lesen` liest nur. `verarbeiten(|datei| …)` übergibt jede lesbare Nachricht
  (älteste zuerst) an den Aufrufer; erst wenn der `Ok` liefert (Cloud hat quittiert), werden TDX
  und Anhang gelöscht. Bei `Err` bleibt die Datei liegen und die Runde endet, damit eine spätere
  Änderung keine frühere überholt. Defekte Dateien wandern nach `FEHLER`. Dateien, die jünger als
  `mindestalter` (2 s) sind, bleiben bis zum nächsten Lauf liegen.
- Versand: `senden("THEORG", …)` schreibt in den Eingang von THEORG. Name `PH` + 6 Zeichen (8.3),
  eindeutig im Zielordner. Erst Anhang (`.TMA` → `.PDF` …), dann Nachricht (`.TMP` → `.TDX`), jeweils
  mit `fsync` und Umbenennen. Ohne vierstellige `SenderVerification` wird nichts geschrieben.

## Abbildung auf Connector v2

`abbilden(&nachricht, &optionen)` liefert `patienten`, `mitgliedschaften`, `checkins` und `hinweise`.
Abgebildet wird nur, wenn der Sender `THEORG` ist; andere Provider tragen in
`CSenderInternalIdentification` ihre eigene Nummer.

| v2 | THEDEX |
|---|---|
| `PatientV2.pvs_id` | `CSenderInternalIdentification` (THEORG-Personennummer) |
| `vorname`, `nachname` | `CFirstName`, `CLastName` |
| `geburtsdatum` | `CBirthday` (`TTMMJJJJ` → `JJJJ-MM-TT`) |
| `geschlecht` | `CSex`: `W`/`F` → `w`, `M` → `m`, `D`/`X` → `d` |
| `telefon`, `telefon_mobil`, `email` | `CTelephone`, `CellularPhone`, `eMail` |
| `strasse`, `plz`, `ort` | `CStreet`, `CZip`, `CTown` |
| `kostentraeger` | `InsuranceStatus` beginnt mit `P` → `selbstzahler`; sonst 9-stellige IK in `InsuranceID` → `gkv`; sonst leer |
| `checkin_id` (zusätzlich) | `CCheckInID` |
| `externe_id` (zusätzlich) | `CExternalIdentification` |
| `geloescht` | `ClientDelete` |
| `geaendert_am` | `MessageDate` + `MessageTime` (Europe/Berlin) |
| `MitgliedschaftV2` | `[ContractData]`: `BeginDate`, `EndDate`, `TerminationDate` → `kuendigung_zum`, `SuspendBeginDate`/`SuspendEndDate` → `ruhezeit_von`/`ruhezeit_bis`, `Description` → `bezeichnung`; `pvs_id` = `<Patient>:<Beginn JJJJMMTT>:<SHA-256 der Bezeichnung, 16 Hex>` |
| `CheckinV2` | `[CheckInOutData]`: `Event` → `in`/`out`, Zeitpunkt aus CheckIn- bzw. CheckOut-Datum/-Zeit (fehlt sie: Nachrichtenzeit), `Description` → `notiz`; `pvs_id` = `<Patient>:<in/out>:<JJJJMMTTHHMM>` |

Bei `ClientChange` zählt `[ClientNewData]`. Leere Felder werden weggelassen (die Cloud
überschreibt mit leeren Werten nichts).

Ausgehend:

- `patient_anlegen` (Vertrag 6.1) → `ClientNew`. `CExternalIdentification` = `praxishub_patient_id`,
  ersatzweise der Idempotenz-Schlüssel. THEORG speichert den Wert und gibt ihn beim Export wieder
  mit; darüber ordnet die Cloud die neue THEORG-Nummer zu.
- `dokument_ablegen` (neu): `patient_pvs_id` oder `vorname`/`nachname`/`geburtsdatum`, `titel`,
  `datum`, `dateiname`, `mime`, `inhalt_base64`, optional `therapeut` → `ClientReport` mit
  `ReportType` aus dem MIME-Typ. THEORG ordnet Personen beim Import über Name, Vorname und
  Geburtsdatum zu, nicht über seine Personennummer; fehlen diese Angaben, nimmt `client_report`
  sie aus dem zuletzt gelesenen Patienten (`bekannt`), sonst Fehler `patient_unbekannt`.
- Geschrieben werden nur Felder mit Inhalt, weil THEORG so eingestellt sein kann, dass leere Felder
  vorhandene Inhalte löschen. Zeichen außerhalb von Windows-1252 werden zu `?`.
- `ThedexFehler::fehlercode()` liefert den passenden Code für die Auftragsquittung.

## Auslegung der Spezifikation

- Die Beispiele der Spezifikation sind nicht ganz einheitlich. Toleriert werden `CTelephon`,
  `CSex=F`, `SenderLanIdetifikation` und `CBankIdentificationCode` (Spec: `CBankIdendificationCode`).
  Geschrieben wird die Schreibweise der Feldbeschreibung.
- Schlüssel und Abschnittsnamen ohne Rücksicht auf Groß-/Kleinschreibung; Werte werden wie bei
  Windows-INI-Dateien an beiden Enden getrimmt. Doppelte Schlüssel: der erste gilt, weitere bleiben
  erhalten. Unbekannte Felder und Abschnitte (`[Cybex]`, `[THEORG]` …) werden mitgeschrieben.
- `[Thedex]` und `[Sender]` sind Pflicht, fehlen sie, wird trotzdem gelesen (Hinweis). Ohne
  `[Message]`/`MessageType` oder ohne `[Client]` ist eine Datei defekt.
- `OEM` = Codepage 850 (DOS Westeuropa). `encoding_rs` kennt CP850 nicht; die Tabelle steht in
  `src/zeichensatz.rs`. Fehlt `MessageCodePageInformation`, gilt ANSI; eine Datei mit UTF-8-BOM wird
  als UTF-8 gelesen.
- Uhrzeit: Die Spezifikation schreibt „SS:MM“ und meint Stunde:Minute.
- Mehrere `[ContractData]`-Abschnitte werden als mehrere Verträge gelesen.
- `ReportFile` ist ein Dateiname im selben Ordner wie die TDX.

## Diagnose: `thedex-probe`

```
thedex-probe C:\THEDEX            # liest C:\THEDEX\PRAXHUB
thedex-probe C:\THEDEX THEORG     # liest den Eingang von THEORG (z. B. eigene Testdateien)
thedex-probe C:\THEDEX PRAXHUB --roh
```

Gibt Provider aus `THEDEX.INI` und je Datei Typ, Sender, Zeichensatz, Anhang und die abgebildeten
v2-Objekte als JSON aus. Löscht, verschiebt und schreibt nichts. Für die Fernwartungssitzung beim
Kunden; die Ausgabe enthält Patientendaten und bleibt auf dem Praxisrechner.

## Tests

`tests/fixtures/` enthält die Beispiele der Spezifikation (`SPEC*.TDX`, `THEDEX.INI`) und
synthetische Grenzfälle (OEM, Vertrag, Löschung, unbekannte Felder, fehlende Pflichtabschnitte,
Binärdatei), Windows-1252 bzw. CP850 mit CRLF. Erzeugt mit `python3 -I tests/fixtures/erzeugen.py
tests/fixtures`. Keine echten Patientendaten.

## Offen für die Sichtung bei Storhas

- **Exportauslöser in THEORG**: Bei welchen Ereignissen schreibt THEORG `ClientNew`, `ClientChange`,
  `ClientInfo`, `ClientDelete`, `ClientCheckInOut` in `PRAXHUB`? Im Parameterblatt ist Export je
  Nachrichtentyp ankreuzbar; ob das je Provider gilt (`[THEORG_PRAXHUB] Export=1`), ist zu prüfen.
- **Import-Einstellungen**: `ClientNew` bei unbekannter Person auf „aufnehmen“, `ClientReport`
  „Angehängten Bericht verarbeiten“ (Modul Elektronische Akte), „Leerinformationen löschen
  Feldinhalte“ aus, Identität über Nachname, Vorname, Geburtsdatum.
- **ContractData**: Trägt eine Nachricht mehrere Verträge oder Vertragsmodule, und in welcher Form?
  Es gibt keine Vertragsnummer; ändern sich Beginn oder Bezeichnung, entsteht eine neue
  Mitgliedschaft. Ob ein beendeter Vertrag aus späteren Exporten verschwindet (dann fehlt die
  Löschmeldung), ist offen.
- **Erstabgleich**: THEDEX meldet Änderungen. Gibt es in THEORG einen Export aller Personen
  (`ClientInfo` für den ganzen Bestand), oder kommt der Erstabgleich aus der Firebird-DB?
- **Sender-Kennung**: `SenderVerification` (vierstellig) und die Freigabe des Providernamens
  `PRAXHUB` bei SOVDWAER beantragen. Bis dahin schreibt der Connector nichts.
- **Neuanlage**: THEORG vergibt die Personennummer erst beim Import. Die Quittung von
  `patient_anlegen` verlangt `patient_pvs_id`; die kennt der Connector erst, wenn THEORG die Person
  wieder exportiert (mit `CExternalIdentification`). Wie lange THEORG dafür braucht und ob der
  Vertrag dafür eine verzögerte Quittung braucht, ist zu klären.
- **Cloud-Seite**: Entitäten `mitgliedschaften` und `checkins`, Auftrag `dokument_ablegen` und die
  zugehörigen Fähigkeiten gibt es im Connector-v2-Vertrag noch nicht.
- **Kostenträger**: Status `7` (BG) und Kürzel wie `PRIVAT`/`BEIHILFE` werden noch nicht als `bg`,
  `pkv` oder `beihilfe` erkannt; erst an echten Exporten festlegen.
