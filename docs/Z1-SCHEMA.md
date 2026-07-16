# Z1-Datenbank — Komplettkartierung

Systematische Kartierung der Z1-SQL-DB (`Z1` auf `srv-fs\z1`), damit wir *alle*
Funktionen von Z1 kennen und gezielt entscheiden können, was Praxishub aus der DB
lesen/spiegeln/heben kann. Ergänzt den funktionalen Katalog in [Z1-DATABASE.md](Z1-DATABASE.md)
und [Z1-BILLING.md](Z1-BILLING.md) um die **vollständige Tabellenlandschaft**.

**Stand:** Inventar (Phase 1) + Profiling der Prioritäts- und Großtabellen (Phase 2).
136 Tabellen, 2 866 Spalten, 324 PK-Spalten, **0 DB-seitige Foreign Keys**. Read-only via
`praxishub_ro`. Rohdaten: `z1_tables.csv`, `z1_columns.csv`, `z1_pks.csv` (Scratchpad).

**Konfidenz-Marker:** ✓ live per Beispielzeilen verifiziert · ~ aus Name/Spalten
erschlossen · ? unklar/ungeprüft.

## DB-weite Konventionen

- Alle Spalten `varchar`. Jede Tabelle hat **`RINFO`** (34, App-Concurrency-Stempel
  `yyyyMMddHHmmssfff`+Arbeitsplatz+Zähler) und **3 Trigger** (Insert/Update/Delete → CDC
  nach `Z1TRIGGER.Z1.Z1TRIGGER296`, CGM-Replikation; rejecten nicht, journalisieren nur).
- **Keine deklarierten FKs** → Beziehungen über Namens-/ID-Konventionen (`PATNR`, `LFD…`,
  `…ID`, `MDID`=Mandant, `LEBID`=Behandler). Die DB erzwingt sie nicht.
- Datum = `JJJJMMTT`. Geld = Cent-Format `e NNN` (Z1-BILLING.md). `PATNR` = 10 rechtsbündig
  space-gepadded → immer `LTRIM(RTRIM())`.
- **Zahn-/Flächen-Befunde** sind positionsbasierte Strings (ein Zeichen je Zahn/Fläche,
  `@`/`0` = leer, FDI-Reihenfolge). Muss man dekodieren.
- Schreiben ist unsupported → **DB = LESE-Weg, VDDS-media/BVS = SCHREIB-Weg**; verifizierte
  Ausnahmen (UPDATE ADR, INSERT PATINFO/BEH/ARCHIV) siehe Z1-DATABASE.md.

---

## A. Patient & Stammdaten

| Tabelle | Zeilen | Was | Praxishub-Relevanz |
|---|--:|---|---|
| `PAT` ✓ | 18 247 | Patientenstamm, PK `PATNR`. Namensschlüssel `KYPATNAME/KYPATVORNAME` indiziert, `GEBDATUM,LPATNR,VXPATIENTUID`(CGM-Cloud-UID),`EXTERNID,GESPERRT,VERSTORBENAM`, Recall-Felder, `ADRIDx` | **Matching-Basis**; Recall-Hebel |
| `ADR` ✓ | 23 893 | Adresse/Kontakt: `NAME,VORNAME,STR,PLZ,ORT,TELEFON1..7,SECUREMAIL,BERUF`, Bank (IBAN…) | Kontakt-/Adress-Writeback (verifiziert) |
| `VDESC` ✓ | 94 464 | Versicherten-/eGK-Stammdaten je Periode: `VERSICHERTENNR,VKNR,Z1KASKUERZEL,KSART,EINLESEDATUM,EGKVSD,GUELTIGBISDATUM` | eGK-Status, Kasse, Gebührenbefreiung |
| `PRUEFNACHWEIS` ~ | 25 037 | eGK-Prüfnachweise (VSDM) je Einlesevorgang | Quartals-/Karten-Status |
| `VDESCENTRY` | 0 | Detailzeilen VDESC (leer) | — |

## B. Versicherung & Kassen (Kataloge)

| Tabelle | Zeilen | Was |
|---|--:|---|
| `Z1KASSEN` ✓ | 1 342 | Kassenverzeichnis der Praxis; `EBZIK`(IK→EEBZ0),`KASSENART` |
| `BKVKASSEN` ~ | 9 014 | Bundesweites GKV-Kassenverzeichnis |
| `KZV` ~ | 25 | KZV-Stammdaten/Punktwerte | `KTRAEGER` 2, `LAND` 1, `BKTKASSEN/BKTORT` 0 (leer) |

## C. Behandlung, Leistungen & Karteikarte

| Tabelle | Zeilen | Was | Praxishub-Relevanz |
|---|--:|---|---|
| `BEH` ✓ | 1 467 030 | **Karteikarte/Leistungshistorie.** Leistungspositionen (`GOART,DMBETRAG,KYLSTNR,ICDCODE`) + Freitextzeilen (`BEHTEXTART='k'`, GOART leer). PK `PATNR+DATUM+BEHSESSION+LFDSESSIONENTRY` | Verlaufsdoku, Notiz-Writeback (verifiziert), Umsatz-/Leistungsanalyse, **Diagnosen** (`BEH.ICDCODE`) |
| `FREITEXT` ✓ | 408 045 | **Interne Kommunikation/Notizen je Patient** (`FREITEXTART='p'`, `BESCHREIBUNG` Freitext, `TODO`) — Team-Chat/Merker, NICHT klinisch/abrechnungsrelevant | Team-Notizen (ggf. spiegeln) |
| `GOCHECK` ✓ | 320 469 | **Abrechnungs-Plausibilitätsregeln** (BEMA/GOZ/GOÄ): Kombinations-/Ausschluss (`A…`/`B…`), `MAXANZ,ZEITRAUM`, Muss-Begründung/-Datum, `SEX,ALTER` — Katalog | (tief) Abrechnungsprüfung |
| `GOTEXTENTRY` ✓ | 317 768 | Zusatz-/Facharzt-Texte je GO-Ziffer — Katalog | Leistungstexte |
| `GO` ~ | 12 320 | Gebührenordnungs-Katalog (BEMA/GOZ, Punktwerte `PWLART`) | Leistungsbewertung |
| `PUNKTWERTE` ✓ | 6 363 | Punktwerte je `PWLART`+`ABDATUM` | € je Punkt |
| `ICD` ✓ | 104 939 | **ICD-10-GM-Katalog** (Kapitel/Gruppe/Subgruppe, `SEKUNDAERCODE,MELDEPFLICHTIG`) — global. Diagnosen je Patient in `BEH.ICDCODE` | Diagnose-Lookup |
| `LBLOCK` ✓ | 21 400 | **Leistungsblock-Kopf** je Plan (`PATNR+LFDPLAN+LFDLBLOCK`, `LBLOCKART,LABOR,BETRAEGE`=gepackte Summen) | Abrechnungs-/Laborstruktur |
| `LBLOCKENTRY` ✓ | 155 724 | **Leistungsblock-Positionen** (`GOART,LSTNR,ANZAHL,EINZELBETRAG,ZAEHNE,BEMERKUNG,LFDPABEFUND`; GOART 3/4=Material/Labor) | Detaillierte Leistungs-/Laborabrechnung |
| `MAC`/`MACENTRY` ✓ | 807/8 752 | **Abrechnungs-Makros** (Leistungsketten-Vorlagen: `MACID`, Positionen) — Katalog | erklärt Leistungseingabe |
| `REZEPT` ✓ | 2 983 | **(e)Rezept-Verordnungen** je Patient (`ICD,PRIVATREZEPT,BTMREZEPTNUMMER,REZEPTSTATUS,SIGNATUR,VERSANDDATUM1-3,DOKUMENTENID`=UUID) | Verordnungs-/eRezept-Historie |
| `MEDIKAMENT`/`ARZNEIMITTEL` ~ | 30/17 | Medikamenten-/Arzneimittelstamm (praxiseigen) | Medikationsplan-Abgleich |
| `GOSUBSUB` 282, `GTEXT` 99, `GOBUDGET` 4 | | GO-Detailkataloge | — |

## D. Befunde (Diagnostik)

| Tabelle | Zeilen | Was | Praxishub-Relevanz |
|---|--:|---|---|
| `ABEFUND` ✓ | 88 272 | **Zahn-/Mundbefund (grafisches Zahnschema)** je Patient: `BEFUND`(32-Zahn-String), `CARIESFLAECHEN,FLGFLAECHEN,INLAYFLAECHEN`(flächenbezogen), `WFKANAELE,VITAL,KRONENART,IMPLANTATART,ZAHNSTEIN,API,PBI,PSI`, Zahnstellung `LAGE*` | **Behandlungsbedarf** (Karies→Potenzial), Prophylaxe-Indizes |
| `PABEFUND` ✓ | 4 092 | **PAR-Befund detailliert**: `TASCHENTIEFE*`(je Fläche), `LOCKERUNGSGRAD,FURKATION,REZESSION,BOP,DIAGNOSE1-6,PASTATUS`, Risiko (`DIABETES,PFPRESSEN`…) | PAR-Hebel (Befund→Antrag) |
| `PARSTATUS` ✓ | 356 | **PAR-Klassifikation** (KZBV 2021): `STADIUM(1-4),GRAD,RKA,CAL,ZAHNVERLUST,VERTEILUNG,DIABETES,RAUCHEN` | PAR-Grading (Antragsgrundlage) |
| `EBEFUND` ✓ | 6 | **Endo-Befund** (Wurzelkanal-Doku je Zahn: `ZAHNNR,AKRBEF,APEX,BMEDEINL,BFUELLMAT/TECH,BSEALER`) — selten genutzt | Endo-Doku |
| `CMD` ✓ | 557 | **CMD-/Funktionsbefund** (Kiefergelenk) + Befund-Vorlagen (`PATNR=0`; `FELDID,SELECT,TXT`) | Funktionsdiagnostik |
| `ROEBEFUND` ✓ | 5 | **Röntgen-Protokoll** (Rechtf. Indikation, `XRAYMS/VOLTAGE,GRAVIDITAET,INDIKATIONSGRUND`) — kaum genutzt (Bildgebung läuft extern/Sidexis) | — |
| `KFOBEFUND` 2, `MESSUNG` 0 | | KFO-Befund (kaum), Messwerte (leer) | — |

## E. HKP / Pläne / EBZ (Genehmigungswege)

| Tabelle | Zeilen | Was | Praxishub-Relevanz |
|---|--:|---|---|
| `ZPLAN` ✓ | 12 487 | Planköpfe (ZE/HKP): `PLANART,KASSENPLAN,ANTRAGSNUMMER,LEBID` | HKP-Tracking (Fall-Status) |
| `ZEHIT`/`ZEHITLST` ✓ | 2 069/13 316 | ZE-Befund/Planungszeilen (Regelversorgung, Festzuschüsse) | HKP-Leistungen/-Beträge |
| `EBZ` ✓ | 4 198 | Elektronischer Antrag: `DOKART`(1=Antrag,3=Antwort,4=Rückfrage),`ZUGESTELLT` | **HKP-Status live** (genehmigt/abgelehnt) |
| `PARHIT`/`PARHITLST` ✓ | 1 045/7 777 | PAR-Fall/-Antrag (`PASTATUS`=kodierter Charting-String, `GUTACHTERDATUM,THERAPIEERG,ANTRAGSNUMMER`) + Leistungszeilen | PAR-Tracking (mit PABEFUND/PARSTATUS) |
| `KBRHIT`/`KBRHITLST` ✓ | 1 964/15 355 | Kieferbruch/KGL-Fall (`VORHZAEHNE,PLANUNGSDATUM,ANTRAGSNUMMER`) + Leistungszeilen | KBR-Tracking |
| `QAHIT` ✓ | 1 126 | **KCH-Quartalsabrechnung** (BEMA konservierend-chirurgisch, KZBV-DTA): Fälle je Quartal, `PKTSUMKC/IP,ZAHONORAR,SUMTOTAL,DTA1-4` | **GKV-Umsatz KCH** (Abrechnungswahrheit) |
| `KFOHIT` 0, `HMPREIS` 0 | | KFO-Pläne (leer — kein KFO) | — |

## F. Abrechnung & Zahlung

| Tabelle | Zeilen | Was | Praxishub-Relevanz |
|---|--:|---|---|
| `BILL` ✓ | 97 117 | Rechnungsköpfe (`PFID=RART`, Behandlungszeitraum, `PLFDFAKT`→FAKT) | Rechnungs-/Forderungsstatus |
| `FAKT` ✓ | 64 229 | Faktura-Buchung: `BETRAG,ZHON,BEGLICHEN,AUSGEBUCHT,MAHNSTUFE,STORNIERT,FAKTDATUM,RART` | Umsatz, offene Posten, Ausfallrechnung-Spiegel |
| `CASH` ✓ | 21 376 | Barzahlungen/Kassenbuch (TSE) | Zahlungen |
| `KONTO` ✓ | 4 413 | **FIBU-Kontenrahmen** (`KTO,KTOKUERZEL,GLIEDERUNG`, Bilanz/GuV, Debitoren) — nicht Einzelzahlungen | Buchhaltung |
| `BILLRULE` ~ | 8 318 | Abrechnungsregeln | — |
| `FBUCHUNG` ~ | 135 | Finanzbuchungen | FIBU |
| `GLKU` ✓ | 1 | **Gläubiger-/Lieferanten-Konten** (`GLKUNAME,ADRID,RABATTPC,SKONTOPC`) | Vendor/Labor-FIBU |
| `MDFIBUPROFIL`/`MDPRAXISPROFIL` ~ | 17/0 | DATEV/FIBU-Exportprofile | Buchhaltungs-Export |
| `BANKEN` 2, `FDIENST` 3 (Factoring/DZR?), `DATEV*`/`BANKOUT`/`ZBON`/`TSEBON`/`GLKUFAKT`/`GLBLOC*` 0 | | Bank/Factoring/DATEV/TSE (großteils leer) | Factoring-Abgleich (DZR) |

## G. Dokumente, Formulare & Archiv

| Tabelle | Zeilen | Was | Praxishub-Relevanz |
|---|--:|---|---|
| `ARCHIV` ✓ | 106 902 | Dokument-Index je Patient (`EXTERNOBJEKTART`=VDDS-Typ, `MMOID,BVS`) | **Dokument-Ablage-Ziel** (Anamnese/Rechnung/Upload sichtbar machen) |
| `FILEPOOL` ✓ | 35 895 | Blob-Store (`FILEDATA varbinary(max)`) — u.a. EEBZ0-HKP-XML, PDFs | HKP-XML-Quelle |
| `EINWILLIGUNG` ✓ | 13 789 | Einwilligungen (`EINWILLIGUNGART,UNTERSCHRIFTDATUM,LFDARCHIV`) | Einwilligungs-Rückschrieb |
| `INFODOKU`/`INFODOKUENTRY`/`INFODOKUTRIGGER` ✓ | 1 076/33 022/1 076 | Aufklärungsdokumente + Nutzung (`DOKUID,ANZCALLS,INFOSTATUS`) + Auslöser | Aufklärungs-Doku |
| `FRAGEBOGEN`/`FRAGEBOGENENTRY` ✓ | 10/394 | Anamnese-Fragebogen-**Vorlagen** (Antworten je Patient via PATINFO) | Anamnese-Struktur |
| `FORMULAR`/`FORMULARLUPB` ~ | 658/658 | Formular-Vorlagen | — |
| `DOKUMENT` ✓ | 8 | Externe Datei-Verweise (`PFAD` auf Platte, `EPAUNIQUEID`) — kaum genutzt | — |
| `EPADOCUMENT` 0 | | ePA-Dokumente (ungenutzt) | ePA-Anbindung (Zukunft) |

## H. Anamnese, Termine, Recall & Aufgaben

| Tabelle | Zeilen | Was | Praxishub-Relevanz |
|---|--:|---|---|
| `PATINFO` ✓ | 83 389 | Patienten-Info-Zeitachse: `ART`(1=Anamnese…5),`INFORMATION`,`LFDANAMNESE,FRAGEBOGEN…` | Anamnese-Writeback (verifiziert) |
| `AUFGABEN`/`HISTAUFGABEN` ✓ | 17 293/210 784 | **Aufgaben/To-dos** je Patient/Team (`KURZTEXT,PRIO,BISDATUM,INTERVALL,QMRELEVANT`) + Historie | offene Patienten-Aufgaben surfacen |
| `HISTRECALL` ✓ | 3 557 | **Recall-Historie** (`RECALLDATUM,RECALLART,BENACHRICHTIGUNG`) — dünn | **Recall-Hebel** (PZR/Prophylaxe) |
| `WARTELISTE` ✓ | 1 090 | Warteliste/Kurzfrist-Termine (`STATUS,LEISTUNG,LEBID,NOTIZ`) — faktisch ungenutzt (zuletzt 2021) | Terminlücken |
| `ETSSTERMIN` ✓ | 0 | **116117-TSS-Termine** (`STATUSTSS,STARTDATUM,HERKUNFT`) — leer/ungenutzt | Termin-Sync (Doctolib extern) |
| `TB`/`TBENTRY` 1 751/6 360, `WORKFLOW`/`WORKSTEP`/`WSFAVORIT` | | Textbausteine, Behandlungs-Workflows | — |

## I. Kommunikation & Telematik (TI)

| Tabelle | Zeilen | Was | Praxishub-Relevanz |
|---|--:|---|---|
| `KOMLEMAIL` ✓ | 6 784 | **KIM-Mail-Store** je Patient (`MAILFROM/TO,SUBJECT,MESSAGEID,DIENSTKENNUNG`). `SUBJECT=EEBZ0_…ZE…` → enthält **EBZ-HKP-Nachrichten** | KIM/EBZ-Alternativquelle (HKP-Einreichung/-Antwort) |
| `GEMATIKEVENT` ✓ | 210 312 | **TI-Konnektor-Events** (`TOPIC=CARD/REMOVED…`, `PARAMETER=CardHandle/EGK`) | Karten-Steck-Events, TI-Health |
| `GEMATIKERROR`/`GEMATIKTRACE` ✓ | 114 797/88 574 | TI-Fehler/-Traces (Konnektor) | TI-Health-Monitoring |
| `Z1POST` ✓ | 1 590 | **PLZ-/Orts-/Straßen-Verzeichnis** (Adress-Autocomplete) — kein Postfach! → gehört zu Referenzkatalogen |

## J. Praxis, Personal & Systemkonfiguration

| Tabelle | Zeilen | Was |
|---|--:|---|
| `PERSONAL` ✓ | 41 | Behandler/Personal (`KUERZEL`, Gematik-Namen) — `LEBID`→hier |
| `LEB` ~ | 18 | Leistungserbringer-Stammsatz |
| `NUMBERPOOL` ✓ | 1 | **Atomare ID-Vergabe** (PATNR/ADRID/LFD…) — kritisch für Neuanlagen |
| `MANDANT` 2, `PRAXIS` 1, `Z1SYSTEM` 1 | | Mandant/Praxis/System-Konfiguration |
| `SCHNITTSTELLEN` ✓ | 0 | Konfigurierte GDT/BDT-Schnittstellen — **leer** (kein Import-Fallback) |
| `GERAETE` ✓ | 89 | **Medizinproduktebuch (MPG)**: Hersteller/Serien-Nr./Garantie/Kontroll-/Störungsdaten; u.a. eHealth-Kartenleser |
| `WORKSTATION` ✓ | 78 | Arbeitsplätze: Kartenleser-Port, Standard-Filter, **`ACTPATNR`**=aktuell geöffneter Patient (= „Variante-A"-Trigger) |
| `PRINTER`/`ROOM` | 876/26 | Drucker (je Arbeitsplatz) / Behandlungsräume |
| `PROGPOOL` ✓ | 222 | **Interne Programm-/Modul-Registry** (jedes Z1-Teilprogramm mit Lizenz, Aufrufzähler, Sperre) |
| `MODULEPOOL` ✓ | 115 | **Käuflicher Lizenz-Baukasten** (Modulname, Lizenz, Preis/Monatspreis) |
| `PARPOOL` ✓ | 17 464 | **Key-Value-Parameter-Store** je Programm (`PROGID,KUERZEL,PARVALUE`) — Systemkonfiguration |
| `SELECTPOOL` ✓ | 1 970 | Konfigurierbare Auswahllisten/Dropdowns |
| `PROTOKOLL` ✓ | 108 779 | **Änderungsprotokoll (Audit/GoBD)** je Patient (`ART,INFO`, z.B. „Lstg. gelöscht") |
| `RECHTE`/`PWGROUPS` | 6/76 | Rollen (Programm-IDs × Permission-Bitmaske) |

## K. Labor, Material, Sterilisation & QM

| Tabelle | Zeilen | Was | Praxishub-Relevanz |
|---|--:|---|---|
| `LABAUFTRAG` ✓ | 2 702 | **Zahntechnischer Laborauftrag** zum Plan (`LFDPLAN/LFDLBLOCK`, `ZAHNFARBE/FORM,TERMIN1-7`=Laborschritte, `AUFTRAGSSTATUS,VERSANDDATUM`) | Labor-Workflow/-status |
| `FLABS`/`ELABOR` 12/0 | | Fremd-/Eigenlabor-Stamm | Laborzuordnung |
| `STERIBUCH`/`SBBESTELL*`/`MATERIALBUCH`/`QUAMADOKU`/`OPS*` 0 | | Sterilgut/Bestellung/Material/QM/OPS — leer/ungenutzt | — |

---

## Kernerkenntnisse für Praxishub

- **Klinischer Bedarf ist lesbar:** `ABEFUND` (Karies/Zahnschema) + `PABEFUND`/`PARSTATUS`
  (PAR-Grading) + `BEH.ICDCODE` (Diagnosen) → **Behandlungs-/Potenzialanalyse** ohne Rateweg.
- **Umsatzwahrheit mehrgleisig:** `QAHIT` (KCH-Quartal), `FAKT`/`BILL` (Rechnungen), `ZEHIT`/
  `PARHIT`/`KBRHIT` (Planarten) — je Achse eine Tabelle, überschneidungsfrei kombinierbar.
- **Termine leben extern:** `ETSSTERMIN`/`WARTELISTE` ungenutzt → Z1 hält keine Terminagenda;
  Praxishub/Doctolib bleibt die Terminquelle (relevant fürs Recall-Matching).
- **Recall ist dünn** (`HISTRECALL` 3 557) → bestätigt den Recall/PZR-Hebel.
- **EBZ/HKP doppelt verfügbar:** DB (`EBZ`/`ZPLAN`) **und** KIM-Store (`KOMLEMAIL`, `EEBZ0_…`).
- **Kein GDT/BDT-Import** (`SCHNITTSTELLEN` leer) → VDDS-media bleibt der Schreibweg.

## Offen (niedrige Priorität, meist Katalog/leer/bekannt)

`PARHIT*`/`KBRHIT*`/`ZEHIT*`-Feldsemantik im Detail; `GO`/`GOSUBSUB` (Katalog); Config-Tabellen
(`MANDANT,PRAXIS,Z1SYSTEM,PROGPOOL…`); die vielen 0-Zeilen-Tabellen (ungenutzte Module:
DATEV, Sterilgut, Material, OPS, ePA, TSE-Bon). Bei Bedarf je Feature vertiefen.
