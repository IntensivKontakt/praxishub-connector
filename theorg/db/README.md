# THEORG-Datenbank — Nachbildung ohne Patientendaten

Dieser Ordner baut die Firebird-Datenbank von THEORG (SOVDWAER) **ohne echte Daten** nach: gleiche
Tabellen, Spalten, Schlüssel, Trigger und Generatoren, gefüllt mit erfundenen Werten, die in Füllgrad,
Länge und Form den echten ähneln. Gegen diese Nachbildung entwickeln und testen wir den THEORG-Adapter,
bevor wir in der Praxis an die echte Datenbank gehen. Vorbild ist das Vorgehen bei Z1
([../../db/README.md](../../db/README.md)), nur dass hier nicht einmal eine Kopie der echten Daten
nötig ist.

Grundlage sind zwei Dateien, die das Erkundungsskript in der Praxis erzeugt
(`S1-erkundung.ps1 -MitProfil -MitDdl`, Walletpass-Repo, `docs/plans/fachrichtungen/sichtung/`):

- `S1-schema_<Rechner>_<Datum>.sql` – Aufbau der Datenbank per `isql -x` (nur Metadaten).
- `S1-profil_<Rechner>_<Datum>.json` – Kennzahlen je Spalte (Format 1, siehe unten), keine Werte.

Kartierung der Bedeutung (welche Tabelle ist was): [../../docs/THEORG-SCHEMA.md](../../docs/THEORG-SCHEMA.md).

## Was committet wird und was nie

| Datei | Inhalt | Committet? |
|---|---|---|
| `theorg-schema.sql` | Aufbau der THEORG-Datenbank (Ausgabe `-MitDdl`), nach Prüfung durch die Praxis | ja |
| `theorg-profil.json` | Kennzahlen (Ausgabe `-MitProfil`), nach Prüfung durch die Praxis und `fuellen.py --nur-pruefen` | ja |
| `regeln.json` | Verweise und Bedeutungen aus der Kartierung (Muster `regeln.beispiel.json`) | ja |
| `beispiel-schema.sql`, `beispiel-profil.json`, `regeln.beispiel.json` | **erfundenes** THEORG-ähnliches Schema mit Profil, zum Testen der Kette | ja |
| `docker-compose.yml`, `docker-compose.port.yml`, `nachbilden.sh`, `fuellen.py` | Werkzeuge | ja |
| `daten/` (Datenbankdateien der Container, erzeugte `*-daten.sql`) | nur erfundene Daten, aber groß und jederzeit neu erzeugbar | nein (`.gitignore`) |
| THEORG-Sicherungen (`*.fbk`, `*.gbk`), echte Datenbankdateien (`*.fdb`), Exporte mit Inhalten | echte Patientendaten | **nie**, sie verlassen die Praxis nicht |

Die Textausgabe des Erkundungsskripts (`S1-erkundung_*.txt`) enthält Pfade und Rechnernamen der Praxis;
sie gehört in den Sichtungsbericht, nicht in dieses Repo.

## Ablauf

1. **In der Praxis** (Vorab-Sitzung, siehe Leitfaden S1 im Walletpass-Repo):
   `S1-erkundung.ps1 -Datenbank "localhost/3051:<Alias>" -MitProfil -MitDdl`, optional `-MitCodewerten`
   und `-ProfilMaxZeilen 200000` bei großen Datenbanken. Die Praxis sieht die Dateien durch und gibt sie frei.
2. **Prüfen:**
   ```bash
   python3 fuellen.py S1-profil_<Rechner>_<Datum>.json --nur-pruefen
   ```
   prüft das Format streng und die Datenschutzgrenzen (Codewerte nur ≤ 20 Werte, ≤ 3 Zeichen, nicht bei
   Personen-, Freitext- oder Verweisspalten, nicht wenn alle Werte verschieden sind). Codewerte zusätzlich
   von Hand ansehen.
3. **Ablegen:** als `theorg-schema.sql` und `theorg-profil.json` in diesen Ordner, committen.
4. **Nachbilden** (Docker, Firebird-Version wie in `theorg-profil.json` unter `firebird.version`):
   ```bash
   python3 fuellen.py theorg-profil.json --schema theorg-schema.sql --regeln regeln.json \
       --skala 0.05 --ausgabe daten/theorg-daten.sql --bericht
   FB_TAG=v3.0.10 ./nachbilden.sh --neu theorg-schema.sql daten/theorg-daten.sql
   docker compose exec firebird bash      # isql: /usr/local/firebird/bin/isql localhost:/firebird/data/theorg.fdb
   ```
5. **Kartieren und verfeinern:** Erkenntnisse in [../../docs/THEORG-SCHEMA.md](../../docs/THEORG-SCHEMA.md)
   eintragen, Verweise und Bedeutungen in `regeln.json`, dann Schritt 4 wiederholen.

Schnelltest der Kette ohne Praxis:

```bash
python3 fuellen.py beispiel-profil.json --schema beispiel-schema.sql --regeln regeln.beispiel.json \
    --ausgabe daten/beispiel-daten.sql
./nachbilden.sh --db beispiel --neu beispiel-schema.sql daten/beispiel-daten.sql
```

## Firebird im Docker

`docker-compose.yml` startet `jacobalberty/firebird` mit fester Version (`FB_TAG`, Standard `v3.0.10`;
außerdem z. B. `v2.5.9-ss`, `v4.0.2`). Die Version muss zu THEORG passen: Firebird 3 öffnet keine
2.5-Datenbank (anderes Dateiformat ODS), und das Schema aus `isql -x` ist versionsabhängig. Jede Version
legt ihre Dateien in `daten/<FB_TAG>/` ab.

| `firebird.version` im Profil | ODS | `FB_TAG` |
|---|---|---|
| 2.5.x | 11.2 | `v2.5.9-ss` |
| 3.0.x | 12.0 | `v3.0.10` |
| 4.0.x | 13.0 | `v4.0.2` |

Es werden keine Ports veröffentlicht. Zugriff über `docker compose exec firebird …` oder aus Containern im
selben Docker-Netz (Host `firebird`, Port 3050). Für Werkzeuge auf dem eigenen Rechner zusätzlich
`docker-compose.port.yml` (bindet nur an `127.0.0.1`). SYSDBA-Passwort: `FB_PASSWORT` (Standard
`nachbildung`, nur für diese Nachbildung). Zwei Versionen gleichzeitig: je Version `COMPOSE_PROJECT_NAME`
setzen.

## `nachbilden.sh`

Legt aus dem Schema eine leere Datenbank an (Dialekt, Seitengröße und Zeichensatz aus dem Kopf von
`isql -x`), spielt das Schema ein und optional die Daten aus `fuellen.py`. Fehlgeschlagene Anweisungen
werden gezählt und angezeigt; `--streng` bricht dann ab. `--neu` ersetzt eine vorhandene Datei gleichen
Namens, `--db` wählt den Namen. isql läuft mit `LANG=C.UTF-8`, sonst verliert isql 3 Umlaute aus der
Eingabe.

## `fuellen.py`

Python 3.8+, nur Standardbibliothek; `firebird-driver` nur für `--direkt`.

- **Zeilen:** Zeilenzahl aus dem Profil × `--skala` (Standard 0,05), mindestens `--min-zeilen`, höchstens
  `--max-zeilen`; leere Tabellen bleiben leer.
- **Je Spalte:** Füllgrad (`gefuellt`), Anzahl verschiedener Werte (anteilig), Längen, Formklassen
  (nur Ziffern, Datum als Text, Geldformat `e    6426`, Text mit Buchstaben), rechts-/linksbündiges
  Auffüllen, Verteilung über die Jahre, Codewerte mit Häufigkeit. Namen, Straßen, Orte und Texte sind
  Fantasiewerte (Namen aus Silben, E-Mail-Adressen auf `beispiel.invalid`).
- **Aus dem Schema** (`--schema`): Primär- und Unique-Schlüssel (eindeutige Werte), deklarierte
  Fremdschlüssel (Werte aus der Zieltabelle, Zieltabelle wird zuerst gefüllt), `CHECK (… IN (…))`
  auch über Domänen, berechnete Spalten (werden ausgelassen), `CHARACTER SET OCTETS`.
- **Aus den Regeln** (`--regeln`, JSON, Muster `regeln.beispiel.json`):
  - `verweise`: `{"von": "TERMIN.PATNR", "nach": "PATIENT.PATNR"}` – nicht deklarierte Beziehungen.
  - `spalten`: `{"TERMIN.DAUER": {"werte": {"20": 55, "30": 25}}}` feste Verteilung (`"<null>"` = NULL)
    oder `{"art": "vorname" | "nachname" | "strasse" | "ort" | "email" | "text" | "laufend" | "auslassen"}`;
    bei `laufend` optional `"breite"` und `"rechtsbuendig": true`.
  - `zeilen`: feste Zeilenzahl je Tabelle; `auslassen`: Tabellen, die leer bleiben.
- **Trigger** werden beim Einspielen abgeschaltet und danach wieder eingeschaltet (`--mit-triggern` lässt
  sie an). Sonst schreiben Protokoll-Trigger Zeilen dazwischen oder überschreiben IDs.
- **Generatoren** werden auf den Stand aus dem Profil gesetzt (mindestens die größte erzeugte ID).
- **Ausgabe:** SQL-Datei in UTF-8 (`--ausgabe`, Kopfzeile `-- isql-zeichensatz: UTF8`) oder direkt in die
  Datenbank (`--direkt localhost/3050:/firebird/data/theorg.fdb`, Passwort aus `ISC_PASSWORD`, Client-
  bibliothek mit `--fbclient`; braucht `pip install firebird-driver`). `--seed` macht die Daten
  reproduzierbar, `--bericht` zeigt Spalten, deren Füllgrad stark vom Profil abweicht.

### Grenzen

- Beziehungen, die THEORG nicht deklariert, kennt das Skript erst nach der Kartierung (`regeln.json`).
  Bis dahin passen Patientennummern in Termintabellen nicht zu Patienten.
- Das Profil hat für Zahl- und Datumsspalten bewusst kein Minimum/Maximum. Zahlen sind daher frei
  erfunden, Datumswerte folgen nur der Jahresverteilung.
- Formklassen und Füllgrad werden je verschiedenem Wert gewählt; Anteile stimmen ungefähr, nicht exakt.
- Bei Zeichensatz `NONE` erzeugt das Skript nur ASCII (ä → ae), weil Firebird dort Bytes ungeprüft
  speichert und unklar ist, wie THEORG sie kodiert.
- BLOBs werden gekürzt (Text bis 2000 Zeichen, binär bis 256 Bytes).
- UDF-Bibliotheken (`DECLARE EXTERNAL FUNCTION`) liegen nicht im Container; Trigger oder Prozeduren,
  die sie aufrufen, scheitern beim Ausführen.
- Die Nachbildung zeigt Aufbau und Datenformen, nicht die Fachlogik von THEORG (Abrechnung, Prüfregeln
  der Oberfläche). Der Schreibweg eines Termins wird in der Praxis beobachtet (Leitfaden S1, Testtermin)
  und gegen eine Entwicklungskopie innerhalb der Praxis erprobt.

## Format `S1-profil_*.json` (Format 1)

```json
{"werkzeug":"S1-erkundung","format":1,"erstellt":"2026-11-20T10:00:00+01:00",
 "firebird":{"version":"3.0.10","dialekt":3,"zeichensatz":"WIN1252","ods":"12.0"},
 "tabellen":[{"name":"T","zeilen":123,"stichprobe":null,
   "spalten":[{"name":"C","typ":"VARCHAR(10)","nullbar":true,"gefuellt":120,"verschieden":50,
     "laenge_min":1,"laenge_max":10,
     "formen":{"ziffern":0,"datum_jjjjmmtt":0,"datum_ttmmjjjj":0,"rechtsbuendig":0,"linksbuendig":0,"buchstaben":0,"geld":0},
     "jahre":null,"codewerte":null}]}],
 "generatoren":[{"name":"G","wert":1}],
 "trigger":[{"name":"","tabelle":"","typ":"","aktiv":true}],
 "prozeduren":[{"name":""}]}
```

- `zeilen`: alle Zeilen der Tabelle; `stichprobe`: Anzahl ausgewerteter Zeilen, wenn mit
  `-ProfilMaxZeilen` nur die ersten n gezählt wurden, sonst `null`. Alle Spaltenkennzahlen beziehen sich
  auf die ausgewerteten Zeilen.
- `gefuellt`: nicht NULL und nach `TRIM` nicht leer (BLOB: Länge > 0). `verschieden`: verschiedene gefüllte
  Werte nach `TRIM` (`null` bei BLOB). `laenge_min/max`: Zeichen nach `TRIM`, bei BLOB Bytes, sonst `null`.
- `formen`: nur bei Textspalten, sonst `null`. `rechtsbuendig` = führende Leerzeichen, `linksbuendig` =
  Leerzeichen am Ende (bei `CHAR` heißt das: kürzer als deklariert), `datum_ttmmjjjj` auch mit Punkten,
  `geld` = Buchstabe + Ziffern (`e    6426`) oder Zahl mit zwei Nachkommastellen.
- `jahre`: Anzahl je Jahr bei `DATE`/`TIMESTAMP` und bei Textspalten, die zu mindestens 80 % wie ein Datum
  aussehen; sonst `null`. `codewerte`: Liste `{"wert", "anzahl"}` (nur mit `-MitCodewerten`, `wert` `null`
  = NULL), sonst `null`.
- `typ` wie im Katalog (`CHAR(10)`, `NUMERIC(9,2)`, `BLOB SUB_TYPE TEXT` …); Domänen sind aufgelöst.
- Trigger-/Prozedurquelltext steht nur in der Textausgabe.

## Getestet

Auf dem Dev-Server (Wegwerf-Container, eigenes Docker-Netz, keine Host-Ports) mit Firebird 2.5.9, 3.0.10
und 4.0.2: Beispielschema nachbilden, mit `fuellen.py` füllen, `S1-erkundung.ps1 -MitProfil -MitCodewerten
-MitDdl` (PowerShell 7 im Container, isql der jeweiligen Version) gegen die Nachbildung laufen lassen und
jede Kennzahl im Profil mit den erzeugten Daten verglichen (714 Kennzahlen, keine Abweichung). Außerdem
Rundweg Profil → Nachbildung → Profil und `--direkt` mit `firebird-driver`.
