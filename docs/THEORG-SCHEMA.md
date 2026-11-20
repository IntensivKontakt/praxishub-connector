# THEORG-Datenbank — Kartierung (Vorlage)

Kartierung der Firebird-Datenbank von THEORG (SOVDWAER) für den THEORG-Adapter. Muster:
[Z1-SCHEMA.md](Z1-SCHEMA.md) (Tabellenlandschaft), [Z1-DATABASE.md](Z1-DATABASE.md) (Zugang, Konventionen,
Lese-/Schreibwege), [PVS-ARCHITEKTUR.md](PVS-ARCHITEKTUR.md) (Fähigkeits-Domänen).

**Stand:** Vorlage, noch nicht ausgefüllt. Quellen zum Ausfüllen:

- `theorg/db/theorg-schema.sql` – Aufbau (`S1-erkundung.ps1 -MitDdl`)
- `theorg/db/theorg-profil.json` – Kennzahlen je Spalte (`-MitProfil`, Format in [../theorg/db/README.md](../theorg/db/README.md))
- Textausgabe der Erkundung (Trigger- und Prozedurquelltext, Generatorstände) und der Sichtungsbericht
  `S1-bericht.md` im Walletpass-Repo (Testtermin, Schreibweg, THEDEX)
- Nachbildung `theorg/db/` zum Ausprobieren von Abfragen

**Konfidenz-Marker:** ✓ in der Praxis am Testpatienten bzw. in der Oberfläche bestätigt · ~ aus Name,
Profil oder Quelltext erschlossen · ? unklar

Was hier steht, enthält keine Patientendaten: Tabellen- und Spaltennamen, Formate, Codes, Zeilenzahlen.

---

## 1. Zugang

| | |
|---|---|
| Server, Port | `localhost/3051` (THEORG-Standard 3051, Events vermutlich 3052) |
| Firebird-Version, ServerMode | |
| Datenbankdatei, Alias | |
| Zeichensatz, Dialekt, ODS | |
| Mandanten | 3 (Pilotpraxis) – eine Datenbank oder je Mandant eine? |
| DB-Benutzer von THEORG | |
| Fundstelle der THEORG-Zugangsdaten (ohne Wert) | |
| Connector-Benutzer | `PRAXISHUB_LESEN`, angelegt mit `S1-lesebenutzer.ps1 -Lesen` (nur SELECT; neue Tabellen nach Updates erneut freigeben) |
| Benachrichtigung anderer Arbeitsplätze | `POST_EVENT` in Triggern? Namen der Events: |
| Sicherung, Wartungsfenster, nächtliche Jobs | |
| Entwicklungskopie in der Praxis | `gbak`-Rücksicherung als zweite Datenbank auf dem Hauptrechner (Leitfaden S1) |
| Schemaversion für einen Fingerabdruck | Versionstabelle? Generator? |

## 2. Konventionen (gelten für alle Tabellen)

Fallen-Checkliste aus Z1. Jede Zeile einmal für THEORG beantworten; die Spalte „Woran erkennbar“ sagt,
wo man ohne Patientendaten nachsieht.

| # | Falle | Bei Z1 | Woran erkennbar | THEORG | |
|---|---|---|---|---|---|
| 1 | Zahlen und Datum als Text | alle Spalten `varchar`, Datum `JJJJMMTT` | Profil: `formen.ziffern`, `datum_jjjjmmtt`, `datum_ttmmjjjj` bei `CHAR`/`VARCHAR` | | |
| 2 | Auffüllen mit Leerzeichen | `PATNR` 10 Zeichen rechtsbündig, immer `LTRIM(RTRIM())` | Profil: `rechtsbuendig`, `linksbuendig` | | |
| 3 | Geldformat, Fixkomma | Präfix `e` + Cent (`e     6426` = 64,26 €), `ANZAHL` ×100, `FAKTOR` ×10000 | Profil: `formen.geld`; `NUMERIC(p,s)` statt Text? Zahl-Spalten ohne Nachkommastellen für Beträge | | |
| 4 | Stornos nicht negativ | Betrag bleibt positiv, Flag `STORNIERT='1'` | Codewerte von Storno-/Statusspalten; Testbeleg in der Sichtung | | |
| 5 | ID-Vergabe zentral oder per Generator | zentrale Zählertabelle `NUMBERPOOL` | Generatoren und `GEN_ID` im Trigger-Quelltext; Zählertabelle mit einer Zeile; Diff beim Testtermin | | |
| 6 | Änderungsstempel, Concurrency | `RINFO` (Zeit + Arbeitsplatz + Zähler) von der App gesetzt | Spalten wie `GEAENDERT`, `AENDERUNG`, `VERSION`, `TIMESTAMP` auf vielen Tabellen; Diff beim Verschieben | | |
| 7 | Trigger nur Protokoll oder auch Prüfung | 3 Trigger je Tabelle, nur Journal, lehnen nichts ab | Trigger-Quelltext: `EXCEPTION`, `POST_EVENT`, `INSERT INTO <Protokoll>` | | |
| 8 | Mandanten-/Filialspalte | `MDID`, `PRAXISID` in fast allen Bewegungstabellen | gleichnamige Spalte in vielen Tabellen, `verschieden` = Mandantenzahl | | |
| 9 | Konfiguration und Modulregister in der DB | `PARPOOL` (Schlüssel-Wert), `PROGPOOL`/`MODULEPOOL` | Tabellen mit Schlüssel-/Wert-Spalten; Codewerte; Modulnamen | | |
| 10 | Arbeitsplatz-Tabelle mit geöffnetem Patienten | `WORKSTATION.ACTPATNR` | Tabelle mit wenigen Zeilen ≈ Zahl der Arbeitsplätze, Spalte mit Patientenbezug | | |
| 11 | Sammelkonten, Pseudo-IDs | Pseudo-`PATNR` `0kz`/`0kp`/`0kb` für Sammelrechnungen | Profil: Patientennummern mit Buchstaben in sonst numerischen Spalten | | |
| 12 | Gespeicherten Wert lesen statt nachrechnen | ZE-Honorar nicht rekonstruierbar, `FAKT.ZHON` lesen | Beträge in Belegtabellen vs. Positionen × Preis | | |
| 13 | Status gegen die Oberfläche abgleichen | `EBZ.ZUGESTELLT` 1/0, `DOKART` 1/3/4 | Codewerte; jeden Code einmal in der THEORG-Oberfläche zuordnen | | |
| 14 | Keine deklarierten Fremdschlüssel | 0 FKs, Beziehungen über Namen | Abschnitt Schlüssel der Erkundung; Verweise in `theorg/db/regeln.json` eintragen | | |
| 15 | Löschen als Flag | `GESPERRT`, `DEAKTIVIERTDATUM` | Codewerte von Spalten wie `GELOESCHT`, `AKTIV`, `STATUS`; Diff beim Löschen des Testtermins | | |
| 16 | Schreibweg sanktioniert? | DB lesen, Dokumente über VDDS-media | THEDEX (`ClientNew`, `ClientReport`), Herstellerposition, Leitfaden §5 | | |

## 3. Praxishub-Funktionen und die Bereiche, die sie brauchen

| Praxishub-Funktion | braucht aus THEORG | Bereich (§4) |
|---|---|---|
| Online-Buchung von Geräten und Sprechstunden mit Vertragsprüfung | Terminplan-Spalten/Ressourcen, freie Zeiten (Arbeitszeiten, Abwesenheiten, belegte Termine), Mitgliedschaft und Vertragsmodule des Patienten | 4.2, 4.3, 4.5 |
| Website-Formulare: Patient zuordnen, Dokument ablegen | Patientenstamm (Name, Geburtsdatum, Kontakt) zum Abgleich, Ablage in der elektronischen Akte | 4.1, 4.8, 4.9 |
| Ansicht „Aktuelle Patienten“ | Termine des Tages mit Status „wahrgenommen“, offene Zahlungen, Verordnungsstand (Einheiten x von y, Zuzahlung) | 4.3, 4.4, 4.7 |
| Kursverwaltung | Kurse, Teilnehmer, Termine, Plätze, Rehasport-/Funktionstrainings-Verordnungen | 4.6, 4.3 |
| Mitglieder-Ansprache nach Check-ins | Mitglieder, Verträge, Check-in-Historie (DB oder THEDEX `ClientCheckInOut`) | 4.5, 4.9 |

## 4. Bereiche

Je Bereich eine Zeile pro Tabelle. Zeilenzahlen aus `theorg-profil.json`; „Wichtige Spalten“ mit Format
(Profil-Formklassen) und Codes (Codewerte bzw. Oberfläche). Suchbegriffe sind Vermutungen für die
Namenssuche im Katalog.

### 4.1 Patienten

Braucht: Website-Formulare (Zuordnung), Aktuelle Patienten, Buchung. Suchbegriffe: PATIENT, PAT, KUNDE,
PERSON, ADRESS, KONTAKT.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Patientennummer (Format, Vergabe), Name/Geburtsdatum für den Abgleich, Telefon/E-Mail, Kostenträger,
Sperr-/Lösch-Kennzeichen, Mandant, Fremdnummer aus THEDEX (`CExternalIdentification`).

### 4.2 Therapeuten, Personal, Arbeitszeiten

Braucht: Buchung (freie Zeiten), Aktuelle Patienten (Behandler). Suchbegriffe: MITARB, THERAP, PERSONAL,
BENUTZER, BEHANDLER, ARBEITSZEIT, ZEITRASTER, URLAUB, ABWESEN, SPERR, DIENSTPLAN.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Bezug Therapeut ↔ Terminplan-Spalte, Zeitraster je Therapeut, Abwesenheiten, Qualifikationen je
Leistung, Arbeitszeitverwaltung (Modul) getrennt vom Zeitraster des Terminplans?

### 4.3 Terminplan

Braucht: Buchung, Aktuelle Patienten, Kursverwaltung. Suchbegriffe: TERMIN, TERM, KALENDER, PLAN, SPALTE,
RAUM, RESSOURCE, GERAET, SERIE, DAUER, WARTE.

**Spalten/Ressourcen** (Pilotpraxis: 23 Spalten)

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

**Termine**

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

**Status** (Codes gegen die Oberfläche abgleichen)

| Code | Spalte | Bedeutung in der Oberfläche | |
|---|---|---|---|
| | | wahrgenommen | |
| | | abgesagt | |
| | | Ausfall/Fehltermin | |
| | | offen | |

**Serien/Dauertermine, Warteliste**

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Datum/Uhrzeit-Format, Dauer, Bindung an Verordnung und Leistung, Gruppentermine, Raum als Spalte oder
eigene Tabelle, Sperren beim offenen Dialog, Live-Aktualisierung anderer Arbeitsplätze (Schreibweg §5).

### 4.4 Verordnungen

Braucht: Aktuelle Patienten (Verordnungsstand), Buchung (Termine für angelegte Rezepte). Suchbegriffe:
REZEPT, VERORD, VO, HEILMITTEL, POSITION, ZUZAHL, BEFREI, ARZT, LANR, ICD, DIAGNOSE.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Einheiten verordnet (y) und erbracht (x) gespeichert oder aus Terminen gezählt? Zuzahlung (Format,
Status bezahlt/offen/befreit), Heilmittel und Positionen, Ausstellungsdatum und Fristen.

### 4.5 Mitglieder, Verträge, Vertragsmodule

Braucht: Buchung mit Vertragsprüfung, Mitglieder-Ansprache. Suchbegriffe: MITGLIED, VERTRAG, MODUL, BEITRAG,
TARIF, LASTSCHRIFT, KUENDIG, RUHE, CHECKIN, CHIP.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: welche Vertragsmodule welche Geräte/Kurse erlauben, Laufzeit, Kündigung, Ruhezeit, Check-in-Historie
(Tabelle oder nur THEDEX), Vertragsabrechnung.

### 4.6 Rehasport, Funktionstraining, Kurse

Braucht: Kursverwaltung. Suchbegriffe: KURS, REHA, FUNKTION, GRUPPE, TEILNEHM, PLATZ, UEBUNGSLEITER.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Kurs ↔ Termine ↔ Teilnehmer, Verordnung Rehasport/Funktionstraining (Einheiten, Zeitraum),
Anwesenheit.

### 4.7 Verkauf, offene Posten

Braucht: Aktuelle Patienten (offene Zahlung). Suchbegriffe: VERKAUF, ARTIKEL, BELEG, RECHNUNG, KASSE,
OPOSTEN, OFFEN, ZAHLUNG, MAHN, STORNO.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Betragsformat (Falle 3), Storno (Falle 4), offener Betrag gespeichert oder errechnet (Falle 12),
Zuzahlungsrechnungen getrennt von Verkauf?

### 4.8 Elektronische Akte, Dokumente

Braucht: Website-Formulare (Dokument ablegen). Suchbegriffe: AKTE, DOKUMENT, DATEI, ANHANG, ARCHIV, BLOB,
SCAN, BRIEF.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Dokument als BLOB in der DB oder Datei mit Pfad? Dokumentarten, Bezug zum Patienten. Kommen
THEDEX-`ClientReport`-Anhänge hier an (§5)?

### 4.9 THEDEX

Braucht: Patienten anlegen und Dokumente ablegen ohne DB-Schreibzugriff, Check-ins. Bibliothek:
[../theorg/thedex/README.md](../theorg/thedex/README.md). Suchbegriffe: THEDEX, PROVIDER, SCHNITTSTELLE,
EXTERN.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Providerverwaltung in der DB? Prüfnummer `SenderVerification` (vierstellig) aus der THEORG-
„Providerverwaltung“, Ablage der Fremdnummer (`CExternalIdentification`), Export-Einstellungen.

### 4.10 Mandanten

Suchbegriffe: MANDANT, PRAXIS, FILIALE, BETRIEB, IK, BSNR.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Mandantenspalte in den Bewegungstabellen (Falle 8), gemeinsame oder getrennte Patientenstämme.

### 4.11 Konfiguration, Module, Arbeitsplätze, Protokoll

Suchbegriffe: KONFIG, EINSTELL, PARAM, OPTION, MODUL, LIZENZ, VERSION, ARBEITSPLATZ, STATION, PROTOKOLL,
LOG, JOURNAL, BENUTZER, RECHTE.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

Offen: Modulregister (Falle 9; Pilotpraxis: Terminplan, Mitgliederverwaltung/Vertragsabrechnung,
Rehasport/Funktionstraining, Elektronische Akte, Arbeitszeitverwaltung, Verkauf, THEDEX, THEORG 2GO),
Schemaversion, Arbeitsplatz mit geöffnetem Patienten (Falle 10), Protokoll-Trigger (Falle 7).

### 4.12 Kataloge

Ärzte, Kostenträger, Heilmittel-/Leistungskatalog, Preise. Suchbegriffe: ARZT, KASSE, KOSTENTR, LEISTUNG,
TARIF, PREIS, POS, KATALOG.

| Tabelle | Zeilen | Schlüssel | Wichtige Spalten (Format, Codes) | Bedeutung | |
|---|--:|---|---|---|---|
| | | | | | |

## 5. Schreibwege

Aus dem Testtermin der Sichtung (`S1-termin-diff.ps1`) und THEDEX. Schreibversuche nur gegen die
Entwicklungskopie in der Praxis.

| Vorgang | Weg (DB/THEDEX/keiner) | Tabellen (+/−/~) | ID-Vergabe | gesetzte Spalten | Trigger, Events | |
|---|---|---|---|---|---|---|
| Termin anlegen | | | | | | |
| Termin verschieben | | | | | | |
| Termin absagen | | | | | | |
| Termin löschen | | | | | | |
| Patient anlegen | THEDEX `ClientNew` oder DB | | | | | |
| Dokument ablegen | THEDEX `ClientReport` oder DB | | | | | |
| Check-in | THEDEX `ClientCheckInOut` (THEORG → Praxishub) | | | | | |

## 6. Leere Tabellen (ungenutzte Module)

Tabellen mit 0 Zeilen zeigen Funktionen, die THEORG kann, die Praxis aber nicht nutzt (wie bei Z1,
[PVS-ARCHITEKTUR.md](PVS-ARCHITEKTUR.md) §4).

| Tabelle(n) | vermutetes Modul |
|---|---|
| | |

## 7. Offene Fragen an SOVDWAER

- Ist lesender Zugriff eines Dienstleisters der Praxis auf die Datenbank erlaubt, und schreibender?
- Gibt es eine Partner-Schnittstelle für Termine (Gateway von TheraConnect/Online-Terminreservierung)?
- Wie oft ändert sich das Schema mit Updates, gibt es eine Versionstabelle?
- Sender-Kennung für THEDEX (Providername, `SenderVerification`).
