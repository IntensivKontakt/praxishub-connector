#!/usr/bin/env bash
# Legt aus einem Schema-SQL (Ausgabe von S1-erkundung.ps1 -MitDdl, also `isql -x`)
# eine leere Firebird-Datenbank im Docker-Container an und spielt optional
# erfundene Daten aus fuellen.py ein. Siehe README.md.
#
#   ./nachbilden.sh [Optionen] SCHEMA.sql [DATEN.sql]
#
# Optionen:
#   --db NAME            Name der Datenbankdatei (Standard: theorg -> /firebird/data/theorg.fdb)
#   --neu                vorhandene Datei dieses Namens vorher loeschen (nur im Container-Ordner daten/)
#   --zeichensatz CS     Standard-Zeichensatz, falls nicht im Schema (Standard: aus dem Schema, sonst NONE)
#   --seitengroesse N    Seitengroesse, falls nicht im Schema (Standard: aus dem Schema, sonst 8192)
#   --streng             Abbruch mit Fehlercode, wenn eine Anweisung fehlschlaegt
#
# Umgebung: FB_TAG (Image-Tag, Standard v3.0.10), FB_PASSWORT (SYSDBA, Standard nachbildung).
set -euo pipefail

HIER="$(cd "$(dirname "$0")" && pwd)"
DB=theorg
NEU=0
STRENG=0
ZEICHENSATZ=""
SEITE=""

hilfe() { sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }
fehler() { echo "FEHLER: $*" >&2; exit 2; }

POS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --db) DB="${2:?}"; shift 2 ;;
    --neu) NEU=1; shift ;;
    --zeichensatz) ZEICHENSATZ="${2:?}"; shift 2 ;;
    --seitengroesse) SEITE="${2:?}"; shift 2 ;;
    --streng) STRENG=1; shift ;;
    -h|--help) hilfe 0 ;;
    -*) fehler "unbekannte Option $1" ;;
    *) POS+=("$1"); shift ;;
  esac
done

SCHEMA="${POS[0]:-}"
DATEN="${POS[1]:-}"
[[ -n "$SCHEMA" ]] || hilfe 2
[[ -f "$SCHEMA" ]] || fehler "Schema $SCHEMA nicht gefunden"
[[ -z "$DATEN" || -f "$DATEN" ]] || fehler "Daten $DATEN nicht gefunden"
[[ "$DB" =~ ^[A-Za-z0-9_-]+$ ]] || fehler "--db nur Buchstaben, Ziffern, _ und -"

# Kopf von `isql -x`: SET SQL DIALECT n; /* CREATE DATABASE '...' PAGE_SIZE n DEFAULT CHARACTER SET cs ... */
DIALEKT=$(grep -m1 -oiE 'SET SQL DIALECT [13]' "$SCHEMA" | grep -oE '[13]$' || true)
DIALEKT=${DIALEKT:-3}
if [[ -z "$SEITE" ]]; then
  SEITE=$(grep -m1 -oiE 'PAGE_SIZE [0-9]+' "$SCHEMA" | grep -oE '[0-9]+' || true)
  SEITE=${SEITE:-8192}
fi
if [[ -z "$ZEICHENSATZ" ]]; then
  ZEICHENSATZ=$(grep -m1 -oiE 'DEFAULT CHARACTER SET [A-Za-z0-9_]+' "$SCHEMA" | awk '{print $4}' || true)
  ZEICHENSATZ=${ZEICHENSATZ:-NONE}
fi
[[ "$ZEICHENSATZ" =~ ^[A-Za-z0-9_]+$ ]] || fehler "Zeichensatz $ZEICHENSATZ ungueltig"
[[ "$SEITE" =~ ^[0-9]+$ ]] || fehler "Seitengroesse $SEITE ungueltig"

export FB_TAG="${FB_TAG:-v3.0.10}"
export FB_PASSWORT="${FB_PASSWORT:-nachbildung}"
DC=(docker compose -f "$HIER/docker-compose.yml")
PFAD="/firebird/data/$DB.fdb"
ZIEL="localhost:$PFAD"

echo "Firebird-Image: jacobalberty/firebird:$FB_TAG"
echo "Schema:         $SCHEMA (Dialekt $DIALEKT, Seitengroesse $SEITE, Zeichensatz $ZEICHENSATZ)"
echo "Datenbank:      $PFAD"

mkdir -p "$HIER/daten/$FB_TAG"
"${DC[@]}" up -d firebird >/dev/null

# Warten, bis der Server Verbindungen annimmt
for _ in $(seq 1 60); do
  if "${DC[@]}" exec -T firebird bash -c 'exec 3<>/dev/tcp/127.0.0.1/3050' 2>/dev/null; then break; fi
  sleep 1
done

fb_isql() {
  # LANG=C.UTF-8: ohne UTF-8-Locale laesst isql 3 Umlaute aus der Eingabe weg
  "${DC[@]}" exec -T -e LANG=C.UTF-8 -e ISC_USER=SYSDBA -e ISC_PASSWORD="$FB_PASSWORT" firebird /usr/local/firebird/bin/isql -q "$@"
}

if "${DC[@]}" exec -T firebird test -e "$PFAD"; then
  if [[ $NEU -eq 1 ]]; then
    "${DC[@]}" exec -T firebird rm -f "$PFAD"
    echo "Vorhandene $PFAD geloescht (--neu)."
  else
    fehler "$PFAD existiert schon. Mit --neu ersetzen oder --db anderen Namen waehlen."
  fi
fi

printf "SET SQL DIALECT %s;\nCREATE DATABASE '%s' PAGE_SIZE %s DEFAULT CHARACTER SET %s;\nCOMMIT;\n" \
  "$DIALEKT" "$ZIEL" "$SEITE" "$ZEICHENSATZ" | fb_isql
echo "Leere Datenbank angelegt."

einspielen() { # $1 Datei, $2 Zeichensatz der Verbindung, $3 Bezeichnung
  local log fehlzahl
  log=$(fb_isql -ch "$2" "$ZIEL" < "$1" 2>&1 || true)
  fehlzahl=$(grep -c 'Statement failed' <<<"$log" || true)
  if [[ "$fehlzahl" -gt 0 ]]; then
    echo "$3: $fehlzahl Anweisung(en) fehlgeschlagen. Erste Meldungen:"
    grep -A4 'Statement failed' <<<"$log" | head -40
    [[ $STRENG -eq 0 ]] || exit 1
  else
    echo "$3: ohne Fehler eingespielt."
  fi
}

# Aufbau: isql -x schreibt UTF8 (S1-erkundung.ps1 ruft isql mit -ch UTF8 auf)
einspielen "$SCHEMA" UTF8 "Schema"

if [[ -n "$DATEN" ]]; then
  CS=$(grep -m1 -oE '^-- isql-zeichensatz: [A-Za-z0-9_]+' "$DATEN" | awk '{print $3}' || true)
  einspielen "$DATEN" "${CS:-UTF8}" "Daten"
fi

printf "SET LIST ON;\nSELECT (SELECT COUNT(*) FROM RDB\$RELATIONS WHERE COALESCE(RDB\$SYSTEM_FLAG,0)=0 AND RDB\$VIEW_BLR IS NULL) AS TABELLEN, (SELECT COUNT(*) FROM RDB\$RELATIONS WHERE COALESCE(RDB\$SYSTEM_FLAG,0)=0 AND RDB\$VIEW_BLR IS NOT NULL) AS VIEWS, (SELECT COUNT(*) FROM RDB\$TRIGGERS WHERE COALESCE(RDB\$SYSTEM_FLAG,0)=0) AS TRIGGER_ANZ, (SELECT COUNT(*) FROM RDB\$GENERATORS WHERE COALESCE(RDB\$SYSTEM_FLAG,0)=0) AS GENERATOREN FROM RDB\$DATABASE;\n" \
  | fb_isql "$ZIEL"
echo "Fertig: $ZIEL im Container (docker compose exec firebird ...)."
