# Anatomie eines vollständigen zahnmedizinischen PVS — Referenz am Beispiel Z1

Ziel dieses Dokuments: **verstehen, was ein komplettes Praxisverwaltungssystem (PVS)
alles können muss und wie es gebaut ist** — unabhängig vom Hersteller. Als konkrete
Referenz dient **Z1 (CGM Dentalsysteme)**, dessen komplette DB wir kartiert haben
(136 Tabellen, `DBRELEASE 296`, Tabellen-Detail in [Z1-SCHEMA.md](Z1-SCHEMA.md)). Die
**Capability-Domänen** (Kap. 3) sind die eigentliche Abstraktion: sie gelten für *jedes*
Dental-PVS (Charly, DS-Win, evident …); nur ihre technische Umsetzung unterscheidet sich.

Ein PVS ist das operative Rückgrat der Praxis: es muss den **gesamten Patienten- und
Geschäftslebenszyklus** abbilden **plus** alle **gesetzlichen/vertraglichen Pflichten**
(KZBV/BEMA, GOZ, TI/gematik, GoBD, MPG, QM, DSGVO). Es ist damit gleichzeitig
Kartei, Abrechnungssystem, Warenwirtschaft, Buchhaltung, Kommunikations-Gateway und
Dokumentenarchiv.

---

## 1. Architekturprinzipien (wie Z1 gebaut ist)

Diese Muster prägen ein gewachsenes PVS und erklären viele „Merkwürdigkeiten":

- **Alles `varchar`, positionsbasierte Kodierung.** Zahn-/Flächenbefunde sind Strings mit
  *einem Zeichen je Zahn/Fläche* in FDI-Reihenfolge (`ABEFUND.BEFUND`, `PABEFUND.TASCHEN…`,
  `WORKSTATION.BEHPAR`). Geld = Cent im `e NNN`-Format, Datum = `JJJJMMTT`.
- **`RINFO`/`CINFO`-Concurrency.** Jede Zeile trägt einen 34-stelligen Änderungs-
  (`RINFO`) bzw. Erstellstempel (`CINFO`): `yyyyMMddHHmmssfff` + Arbeitsplatz + Zähler.
  Optimistisches Sperren ohne DB-Transaktions-Locks.
- **Keine deklarierten Foreign Keys.** Beziehungen laufen über **Konventionen**: `PATNR`
  (Patient), `LFD…` (laufende Nummern je Kontext), `MDID` (Mandant), `PRAXISID` (Filiale),
  `LEBID` (Leistungserbringer/Behandler), `PID` (Personal), `LFDPLAN`/`LFDLBLOCK` (Plan/Block).
  Die Integrität liegt in der Anwendung, nicht in der DB.
- **Konfiguration liegt in der DB, nicht in Dateien:**
  - `PARPOOL` (17 k) = **Key-Value-Parameter** je Programm (`PROGID,KUERZEL,PARVALUE`, z. B.
    `konnektorc=secu_kon`).
  - `SELECTPOOL` (2 k) = konfigurierbare **Auswahllisten/Dropdowns**.
  - `RECHTE` (6) = **Rollen** (Programm-IDs × Permission-Bitmaske); `PWGROUPS`.
  - `WORKSTATION` (78) = **Arbeitsplatz-Profile** (Kartenleser-Port, Standard-Filter,
    `ACTPATNR` = aktuell geöffneter Patient — genau das ist unser „Variante-A"-Trigger).
- **Mehrmandanten-/BAG-fähig.** `MANDANT` (Praxis-Identität, Briefkopf-Stempel, KZV-/
  Abrechnungsnummern, USt-IdNr, SEPA-Gläubiger-ID), `PRAXIS` (Filiale, BSNR, KZV/KV),
  `Z1SYSTEM` (Installation). `MDID`/`PRAXISID` durchziehen fast alle Bewegungstabellen.
- **Das PVS ist selbst modular — und das steht in der DB.**
  - `PROGPOOL` (222) = **interne Programm-/Modul-Registry**: jedes Z1-Teilprogramm
    (`Behandlungsmanager`, …) mit Lizenzstring, Aufrufzähler (`ANZCALLS`), Sperr-/
    Testmodus-Flags. Das ist die Laufzeit-Modulliste des PVS.
  - `MODULEPOOL` (115) = **käuflicher Lizenz-Baukasten** mit Preisen (`Behandlerlizenz mit
    ZANR/LANR — 800 €, 18,15 €/Monat`). Zeigt das kommerzielle Modell eines PVS.
- **Audit/GoBD eingebaut.** `PROTOKOLL` (109 k) = Änderungsprotokoll (wer hat wann welche
  Leistung/Zeile gelöscht/geändert), plus die CDC-Trigger → CGM-Replikationsjournal.
- **Verteilte Client-Server-Architektur.** Ein zentraler SQL-Server, viele Arbeitsplätze
  (`WORKSTATION`, `PROGPATH=C:\CGM\Z1Lokal\exe\`), Zeilen-Locking via
  `LOCKWORKSTATION/LOCKINSTANZ/LOCKZEITPUNKT`.

---

## 2. Datenmodell-Kern (die zentralen Entitäten)

```
PAT (Patient, PK PATNR)
 ├─ ADR (n Adressen: Privat/Rechnung/Arbeit/Kostenträger… via ADRIDP/R/A/K/…)
 ├─ VDESC (n Versicherungs-Perioden, eGK/VSD)  ── PRUEFNACHWEIS (VSDM je Quartal)
 ├─ BEH (Kartei/Leistungen, 1,47 Mio)  ── ICD-Katalog (Diagnosen in BEH.ICDCODE)
 ├─ Befunde: ABEFUND · PABEFUND · EBEFUND · CMD · ROEBEFUND · KFOBEFUND
 ├─ PATINFO (Anamnese/Info-Zeitachse)  ── FRAGEBOGEN(ENTRY) (Vorlagen)
 ├─ FREITEXT (interne Notizen)
 ├─ ZPLAN (HKP-Köpfe)
 │    ├─ ZE:  ZEHIT/ZEHITLST      ├─ PAR: PARHIT/PARHITLST (+PABEFUND/PARSTATUS)
 │    ├─ KBR: KBRHIT/KBRHITLST    ├─ KFO: KFOHIT
 │    ├─ LBLOCK/LBLOCKENTRY (Leistungsblöcke/Labor) ── LABAUFTRAG (Zahntechnik)
 │    └─ EBZ (elektronischer Antrag/Antwort)  ── KOMLEMAIL (KIM-Transport)
 ├─ BILL → FAKT (Rechnung) → CASH/KONTO/FBUCHUNG (Zahlung/FIBU)  ── QAHIT (GKV-Quartal)
 ├─ REZEPT (eRezept)  ── ARCHIV→FILEPOOL (Dokumente)  ── EINWILLIGUNG
 └─ AUFGABEN/WORKFLOW · HISTRECALL · WARTELISTE
Quer: MANDANT/PRAXIS · PERSONAL/LEB · GO/GOCHECK/PUNKTWERTE/KZV (Kataloge) · GEMATIK* (TI)
```

---

## 3. Die Capability-Domänen (was ein PVS können MUSS)

Für jede Domäne: Zweck · Z1-Umsetzung · (ggf.) Standard/Schnittstelle.

### 3.1 Patientenverwaltung & Identität
Stammdaten, mehrere Adressrollen, Beziehungen, Sperr-/Verstorben-Status, Historie.
→ `PAT` (66 Sp.: Recall-Felder, `LASTBEHDATUM`, `VXPATIENTUID`=CGM-Cloud-UID, `EXTERNID`,
`PATBEZIEHUNGEN`, `GESPERRT`), `ADR` (54 Sp., 7 Telefonfelder, Bank, `ANSCHRIFTENZUSATZ`).

### 3.2 Versicherung & eGK/VSDM
Versichertenstammdaten je Periode, eGK-Einlesen, Online-Prüfung (VSDM), Kassenkataloge.
→ `VDESC` (61 Sp.: `VERSICHERTENNR,EGKVSD,GEBUEHRENBEFREITBIS`, Überweiser, Psychotherapie),
`PRUEFNACHWEIS` (VSDM-Prüfnachweis je Quartal), Kataloge `Z1KASSEN/BKVKASSEN/KZV`.
**Standard:** eGK/VSDM über TI-Konnektor.

### 3.3 Klinische Dokumentation (Kartei & Befunde)
Die **Behandlungsdokumentation** — rechtlich Pflicht, GoBD-append-only:
- **Kartei/Leistungen:** `BEH` (1,47 Mio) — Positionen (`GOART/LSTNR/DMBETRAG`) *und*
  Freitext (`BEHTEXTART='k'`), Diagnosen (`ICDCODE`), Verweise auf Plan/Block/Archiv.
- **Befunde je Fachgebiet:** `ABEFUND` (Zahnschema: Karies/Füllung/Krone/Implantat/
  Zahnstein + Indizes API/PBI/PSI), `PABEFUND` (Parodontalstatus: Taschentiefen,
  Lockerung, BOP, Diagnosen) + `PARSTATUS` (Grading Stadium/Grad), `EBEFUND` (Endo/
  Wurzelkanal), `CMD` (Funktion/Kiefergelenk), `ROEBEFUND` (Röntgen-Rechtfertigung),
  `KFOBEFUND` (Kieferorthopädie).
- **Anamnese:** `PATINFO` (ART 1–5) + `FRAGEBOGEN/FRAGEBOGENENTRY` (Vorlagen).
- **Diagnosen:** `ICD` = ICD-10-GM-Katalog (105 k), Zuordnung in `BEH.ICDCODE`.
- **Interne Kommunikation:** `FREITEXT` (Team-Notizen/Merker, *nicht* klinisch).

### 3.4 Leistungserfassung & Gebührenkataloge
Erfassen abrechenbarer Leistungen mit Katalog, Preis, Plausibilität.
→ `GO` (62 Sp. — BEMA/GOZ/GOÄ **und** Laborpreise **und** Materialbestand `MINDESTBESTAND/
BESTAND` = kleine Warenwirtschaft **und** Begründungs-/Dokuvorschläge), `GOCHECK` (320 k
Plausibilitäts-/Kombinationsregeln), `GOTEXTENTRY`, `MAC/MACENTRY` (Makros/Leistungsketten),
`PUNKTWERTE/PWGROUPS`, `TB/TBENTRY` (Textbausteine).

### 3.5 Heil- und Kostenpläne & Genehmigungswesen (KZBV/EBZ)
Der aufwändigste Fach-Workflow: **Befund → Plan → (elektronischer) Antrag → Genehmigung →
Eingliederung → Abrechnung**, je Planart eigene Regeln + Festzuschüsse.
- **Plankopf:** `ZPLAN` (62 Sp.: `PLANART`, Genehmigungs-/Eingliederungs-/`DEAKTIVIERTDATUM`
  → *abgelaufene HKPs*, Reparatur-Flags, `KASSENANTEIL`, `ANTRAGSNUMMER`).
- **Planarten:** ZE `ZEHIT/ZEHITLST` (Zahnersatz, Festzuschüsse `VBPUNKTSUM/NBPUNKTSUM`,
  Laborkosten), PAR `PARHIT/PARHITLST` (+`PABEFUND/PARSTATUS`), KBR/KGL `KBRHIT/KBRHITLST`
  (Kieferbruch/Kiefergelenk), KFO `KFOHIT` (leer — kein KFO).
- **Elektronisch:** `EBZ` (`DOKART` 1=Antrag/3=Antwort/4=Rückfrage, `ZUGESTELLT`) — Transport
  über **KIM** (`KOMLEMAIL`, `SUBJECT=EEBZ0_…ZE…`). **Standard:** KZBV-EBZ.
- **Parameter:** `KZV` (Punktwert, HKP-Faktor, Zuschussfaktoren je regionaler KZV).

### 3.6 Abrechnung (GKV-Quartal, Privat, Rechenzentrum)
- **GKV-Quartalsabrechnung (KZBV-DTA):** `QAHIT` (KCH konservierend-chirurgisch,
  Punktsummen `PKTSUMKC/IP`, `ZAHONORAR`, `DTA1-4`), analog ZE/PAR/KBR über die `…HIT`-DTA.
- **Privatliquidation:** `BILL` (Kopf) → `FAKT` (53 Sp.: `BETRAG/ZHON/ELAB/FLAB`,
  `BEGLICHEN,MAHNSTUFE,STORNIERT,RART`), `BILLRULE` (Regeln).
- **Rechenzentrum/Factoring:** `KTRAEGER` (`DZRS`=DZR), `FDIENst` (Finanzdienst-Abo).

### 3.7 Zahlungsverkehr & Finanzbuchhaltung
Kasse (TSE), offene Posten, Mahnwesen, FIBU-Export (DATEV), SEPA.
→ `CASH` (Kassenbuch/TSE), `KONTO` (FIBU-Kontenrahmen inkl. Debitoren), `FBUCHUNG`
(Buchungen), `MDFIBUPROFIL` (Erlös-/USt-Kontenzuordnung, DATEV), Mahnwesen in
`FAKT.MAHNSTUFE`, SEPA über `MANDANT.GLAEUBIGERID`/`BANKOUT` (leer). `KTRAEGER`/`FDIENST`.

### 3.8 Labor (Zahntechnik)
Eigen-/Fremdlabor, Aufträge, BEB/BEL-Positionen, Terminketten.
→ `LBLOCK/LBLOCKENTRY` (Leistungsblöcke/Positionen, `GOART 3/4`=Material/Labor, `ZAEHNE`),
`LABAUFTRAG` (Auftrag mit `ZAHNFARBE/FORM`, `TERMIN1-7`=Laborschritte, `AUFTRAGSSTATUS`),
`FLABS/ELABOR` (Laborstamm), Laborpreise in `GO`.

### 3.9 Verordnungen
(e)Rezept, Medikamente, BTM, Rezepturen.
→ `REZEPT` (25 Sp.: `PRIVATREZEPT,BTMREZEPTNUMMER,SIGNATUR,VERSANDDATUM1-3`=eRezept,
`DOKUMENTENID`=UUID), `MEDIKAMENT/ARZNEIMITTEL` (Stamm), `REZEPTUR*`/`WIRKSTOFF` (leer).
**Standard:** eRezept über TI.

### 3.10 Dokumente & Bildarchiv
Index + Blob-Store + Bildgebungs-/Scanner-Anbindung + Einwilligungen + Formulare.
→ `ARCHIV` (Index je Patient, `EXTERNOBJEKTART`=VDDS-Typ, **`BVS`** zeigt die angebundenen
Fremdsysteme: `PAVDTQ_Sidexis/DUERR_DBSWIN/…_Scanner`), `FILEPOOL` (Blob `varbinary(max)`,
enthält u. a. EEBZ0-HKP-XML), `EINWILLIGUNG`, `FORMULAR` (Druckvorlagen), `INFODOKU*`
(Aufklärung), `DOKUMENT` (externe Datei-Pfade), `EPADOCUMENT` (ePA, leer).
**Standard:** **VDDS-media** (herstellerübergreifend!) + PACS/GDT für Bildgebung.

### 3.11 Termine & Recall
→ In Z1: `HISTRECALL` (Recall-Historie), `WARTELISTE`, `ETSSTERMIN` (116117-TSS) — letztere
**leer**: der **Terminkalender liegt außerhalb** (hier Doctolib). `PAT`-Recall-Felder
(`LASTRECALLDATUM,BENACHRICHTIGUNG,BESTWOCHENTAG`). Ein „vollständiges" PVS hat einen
eigenen Terminplaner — diese Praxis nutzt einen externen.

### 3.12 Telematik-Infrastruktur (TI)
Das gesetzliche Pflicht-Ökosystem:
- **KIM** (sichere Mail): `KOMLEMAIL` (Transport für EBZ/eArztbrief, `DIENSTKENNUNG`).
- **Konnektor/Kartenterminal:** `GEMATIKEVENT` (Kartenevents `CARD/INSERTED/REMOVED`),
  `GEMATIKERROR/TRACE` (Fehler/Traces), `GERAETE` (eHealth-Terminal, Kartenleser).
- **Identitäten:** `PERSONAL` (HBA: `ICCSN,TELEMATIKID,GEMATIKUSERID`), `MANDANT.GEMATIKMANDANTID` (SMC-B).
- **Fachdienste:** VSDM (3.2), eRezept (3.9), EBZ (3.5), eAU, eArztbrief, ePA (`EPADOCUMENT`).

### 3.13 Aufgaben & Workflow
Aufgabenverwaltung (patient-/teambezogen, wiederkehrend, QM-relevant) + geführte Abläufe.
→ `AUFGABEN/HISTAUFGABEN` (`KURZTEXT,PRIO,INTERVALL,QMRELEVANT`), `WORKFLOW/WORKSTEP`
(geführte Prozesse, z. B. „ZE-Kassenplanung, 17 Schritte"), `WSFAVORIT`.

### 3.14 QM, Hygiene & Medizinprodukte (MPG)
Gesetzliche Dokumentationspflichten.
→ `GERAETE` (35 Sp. = **Medizinproduktebuch**: Hersteller/Serien-Nr./Garantie/Kontroll-
datum/Störungen), `STERIBUCH` (Sterilgut-Doku), `MATERIALBUCH`+`SBBESTELL*` (Warenwirtschaft/
Bestellung), `QUAMADOKU` (QM) — bei dieser Praxis leer, aber vorhanden.

### 3.15 Controlling & Statistik
→ Aggregate aus `BEH`/`FAKT`/`QAHIT`; `GOBUDGET`/`HVMBUDGET` (Budgets), `…STATISTIKSPALTE`.
(Praxishubs Potenzialanalyse/Praxis-Steuerung setzt genau hier an.)

### 3.16 Praxisorganisation & System
→ `PERSONAL/LEB` (Behandler/Benutzer), `RECHTE/PWGROUPS` (Rollen), `WORKSTATION/GERAETE/
PRINTER/ROOM` (Infrastruktur), `MANDANT/PRAXIS/Z1SYSTEM` (Identität), `PROGPOOL/MODULEPOOL`
(Modul-/Lizenz-Registry), `PARPOOL/SELECTPOOL` (Konfiguration), `PROTOKOLL` (Audit).

---

## 4. Was diese Praxis NICHT nutzt (aber ein PVS kann)

Die **leeren Tabellen** sind die Landkarte der ungenutzten Ausbaustufen — sie zeigen den
vollen Funktionsumfang eines PVS:

| Modul (leer) | Fähigkeit |
|---|---|
| `DATEVPAKET/-ENTRY/-ONLINELOG` | DATEV-Online-Übertragung der Buchhaltung |
| `STERIBUCH`,`SBBESTELLUNG/-ENTRY`,`MATERIALBUCH` | Sterilgut-Doku + Warenwirtschaft/Bestellwesen |
| `EPADOCUMENT` | elektronische Patientenakte (ePA) |
| `ETSSTERMIN` | Terminvermittlung 116117 (eTerminservice) |
| `OPS/OPSENTRY` | OPS-Codes (stationäre/ambulante OP-Doku) |
| `REZEPTUR/-ENTRY`,`WIRKSTOFF` | Individualrezepturen |
| `KFOHIT/KFOBEFUND` (fast leer) | Kieferorthopädie |
| `GL*` (`GLKU/GLBLOC/GLKUFAKT`) | Gutschriften-/Gläubiger-(Sammel-)Abrechnung |
| `BANKOUT`,`CASHPAY` | Überweisungsausgang, Kontoauszug-Import |
| `ZBON/TSEBON` | TSE-Kassenbon-Journal |
| `QUAMADOKU`,`MDPRAXISPROFIL` | erweiterte QM-/Praxisprofile |
| `HISTUEBERW` | Überweisungen (vertragsärztlich) |

---

## 5. Implikationen für Praxishub & Multi-PVS

**Die 16 Capability-Domänen sind die Abstraktion** — sie gelten für jedes Dental-PVS.
Charly (solutio, Firebird-DB) und DS-Win (DAMPSOFT) haben *dieselben* Domänen, nur andere
Tabellen. Ein `PvsBackend`-Trait sollte **entlang der Domänen** geschnitten werden
(z. B. `read_hkp_status`, `read_billing`, `write_note`, `file_document`), Z1 ist die
erste Implementierung.

**Was ohne DB-Zugriff geht (PVS-agnostisch, über Standards):**
- **Dokumente/Bilder:** VDDS-media (`ARCHIV.BVS` zeigt: viele Systeme sprechen es schon) —
  unser Schreibweg ist bereits herstellerübergreifend.
- **HKP/Genehmigung:** KZBV-EBZ (standardisiert), Transport KIM.
- **Verordnung/AU/Brief:** eRezept/eAU/eArztbrief über TI-Fachdienste.
- **Geräte-/Labordaten:** GDT/LDT/BEL.

**Was PVS-spezifisch bleibt (direkte Leseschicht, je PVS neu):**
- Status-/Umsatz-/Befund-Reads aus der DB (HKP-Status, Controlling, Behandlungsbedarf,
  Recall). Hier ist Z1s `z1db`-Schicht die Blaupause; für andere PVS je ein Adapter.

**Direkt heraushebbare Datenhebel (aus der Kartierung):** Behandlungsbedarf
(`ABEFUND`-Karies, `PABEFUND/PARSTATUS`-PAR), abgelaufene HKPs (`ZPLAN.DEAKTIVIERTDATUM`),
dünnes Recall (`HISTRECALL`), Umsatzachsen (`QAHIT`/`FAKT`/`ZEHIT`).

---

*Tabellen-Detail und Feldsemantik: [Z1-SCHEMA.md](Z1-SCHEMA.md). Abrechnungs-/Geldformat:
[Z1-BILLING.md](Z1-BILLING.md). Schreib-/Lese-Regeln: [Z1-DATABASE.md](Z1-DATABASE.md).*
