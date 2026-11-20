#!/usr/bin/env python3
"""Erfundene Daten für die THEORG-Nachbildung aus dem Profil von S1-erkundung.ps1.

Grundlage sind nur die Kennzahlen aus ``S1-profil_*.json`` (Format 1): Zeilenzahl,
Füllgrad, Anzahl verschiedener Werte, Längen, Formklassen, Jahre und, falls mit
``-MitCodewerten`` erhoben, die Werte kurzer Codespalten. Namen, Adressen und Texte
sind Fantasiewerte. Es fließen keine echten Daten ein.

Fremdschlüssel kennt das Profil nicht. Deklarierte Schlüssel (PRIMARY KEY, UNIQUE,
FOREIGN KEY, CHECK ... IN) liest das Skript aus dem Schema (``--schema``, Ausgabe von
``isql -x``). Nicht deklarierte Beziehungen und Bedeutungen trägt man nach der
Kartierung in eine Regeldatei ein (``--regeln``, Muster: ``regeln.beispiel.json``).

Aufruf (Beispiele):
    python3 fuellen.py beispiel-profil.json --schema beispiel-schema.sql \\
        --regeln regeln.beispiel.json --skala 0.05 --ausgabe daten.sql
    python3 fuellen.py theorg-profil.json --nur-pruefen
    python3 fuellen.py theorg-profil.json --schema theorg-schema.sql --direkt \\
        localhost/3050:/firebird/data/theorg.fdb      # braucht firebird-driver

Nur Python-Standardbibliothek; ``firebird-driver`` nur für ``--direkt``.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import random
import re
import string
import sys
from collections import defaultdict
from decimal import Decimal

FORMEN = ("ziffern", "datum_jjjjmmtt", "datum_ttmmjjjj", "rechtsbuendig", "linksbuendig", "buchstaben", "geld")
SPALTEN_SCHLUESSEL = ("name", "typ", "nullbar", "gefuellt", "verschieden", "laenge_min", "laenge_max",
                      "formen", "jahre", "codewerte")
# Wie in S1-erkundung.ps1: bei diesen Spaltennamen gibt es nie Codewerte
PERSONEN_MUSTER = re.compile(
    r"(NAME|STRASSE|STRAS|^STR|STR$|PLZ|^ORT|ORT$|WOHNORT|ADRESS|ANSCHRIFT|TEL|FAX|HANDY|MOBIL|MAIL|TEXT|BEMERK|"
    r"NOTIZ|INFO|KOMMENT|MEMO|IBAN|BIC|BLZ|KONTO|GEBURT|GEBDAT|KUERZ|INITIAL|PASS|PWD|KENNW|DIAGNOS|BEFUND|ANAMN)",
    re.I)
# ebenso nie bei Schlüssel-/Verweisfeldern (IDs, Patienten-, Belegnummern)
VERWEIS_MUSTER = re.compile(r"(^ID|ID$|_ID|NR$|NR_|NUMMER|^PAT|PATNR|PATIENT|KUNDE|MITGLIED|PERSON|ARZT|LANR|VERSICH|BELEG|RECHN)", re.I)

# Fantasiewerte (bewusst erfunden bzw. allgemein, keine Personen)
VORNAMEN = ["Anna", "Lena", "Marie", "Sophie", "Clara", "Greta", "Ida", "Frieda", "Jule", "Mila", "Paula", "Romy",
            "Paul", "Jonas", "Felix", "Emil", "Theo", "Anton", "Jakob", "Moritz", "Henri", "Karl", "Lasse", "Ole"]
# Nachnamen aus Silben zusammengesetzt (rund 400 Fantasienamen)
NACHNAMEN = [a + b for a in ("Ahl", "Birk", "Dorn", "Esch", "Feld", "Grün", "Hasel", "Iver", "Kran", "Lind", "Mor",
                             "Nettel", "Oster", "Pfann", "Quern", "Rosen", "Sand", "Tiefen", "Ulmen", "Wend")
             for b in ("brecht", "holm", "feld", "wald", "kamp", "hoff", "berg", "loh", "witt", "mar", "stedt",
                       "beck", "stiel", "heim", "vos", "brunn", "horst", "sang", "hausen", "mann")]
STRASSEN = ["Lindenweg", "Am Mühlbach", "Birkenallee", "Gartenstraße", "Hasenpfad", "Am Kiefernhang", "Wiesenring",
            "Ulmenstraße", "Talblick", "Sonnenhof", "Mühlenkamp", "Rosenstieg"]
ORTE = [a + b for a in ("Neu", "Klein", "Ober", "Nieder", "Alt", "Groß", "Unter", "Hohen")
        for b in ("haldern", "wendorf", "birkenfelde", "hain", "talheim", "rosenau", "mühlhausen", "eschenbrück",
                  "lindenhagen", "wiesental", "kirchdorf", "steinbach")]
WOERTER = ["Behandlung", "Übung", "Kontrolle", "Termin", "Rücken", "Knie", "Schulter", "Massage", "Gerät", "Kurs",
           "Hinweis", "bitte", "nach", "vor", "Woche", "Rezept", "Folge", "Wärme", "Gruppe", "Probe", "neu", "alt"]
WOERTER_ASCII = ["Behandlung", "Kontrolle", "Termin", "Ruecken", "Knie", "Schulter", "Massage", "Geraet", "Kurs",
                 "Hinweis", "bitte", "nach", "vor", "Woche", "Rezept", "Folge", "Gruppe", "Probe", "neu", "alt"]


class Fehler(Exception):
    pass


# ---------------------------------------------------------------------------
# Profil laden und prüfen
# ---------------------------------------------------------------------------

def _ist_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


def profil_pruefen(p):
    """Prüft Format 1 streng. Liefert (fehler, hinweise) als Listen von Texten."""
    fehler, hinweise = [], []
    if not isinstance(p, dict):
        return ["Profil ist kein JSON-Objekt"], []
    erwartet = ("werkzeug", "format", "erstellt", "firebird", "tabellen", "generatoren", "trigger", "prozeduren")
    for k in erwartet:
        if k not in p:
            fehler.append(f"Schlüssel '{k}' fehlt")
    for k in p:
        if k not in erwartet:
            hinweise.append(f"unbekannter Schlüssel '{k}' auf oberster Ebene")
    if fehler:
        return fehler, hinweise
    if p["werkzeug"] != "S1-erkundung":
        fehler.append("werkzeug ist nicht 'S1-erkundung'")
    if p["format"] != 1:
        fehler.append(f"format {p['format']!r} wird nicht unterstützt (erwartet 1)")
    fb = p["firebird"]
    if not isinstance(fb, dict) or set(fb) != {"version", "dialekt", "zeichensatz", "ods"}:
        fehler.append("firebird braucht genau version, dialekt, zeichensatz, ods")
    elif fb["dialekt"] not in (1, 3, None):
        fehler.append("firebird.dialekt muss 1, 3 oder null sein")
    if not isinstance(p["tabellen"], list):
        fehler.append("tabellen ist keine Liste")
        return fehler, hinweise
    for t in p["tabellen"]:
        if not isinstance(t, dict) or set(t) != {"name", "zeilen", "stichprobe", "spalten"}:
            fehler.append(f"Tabelle {t.get('name') if isinstance(t, dict) else t!r}: Schlüssel name, zeilen, stichprobe, spalten erwartet")
            continue
        tn = t["name"]
        for k in ("zeilen", "stichprobe"):
            if t[k] is not None and (not _ist_int(t[k]) or t[k] < 0):
                fehler.append(f"{tn}.{k} muss Zahl >= 0 oder null sein")
        basis = t["stichprobe"] if t["stichprobe"] is not None else t["zeilen"]
        if not isinstance(t["spalten"], list):
            fehler.append(f"{tn}.spalten ist keine Liste")
            continue
        for s in t["spalten"]:
            if not isinstance(s, dict) or tuple(sorted(s)) != tuple(sorted(SPALTEN_SCHLUESSEL)):
                fehler.append(f"{tn}: Spalte {s.get('name') if isinstance(s, dict) else s!r} hat nicht genau die Schlüssel {', '.join(SPALTEN_SCHLUESSEL)}")
                continue
            sn = f"{tn}.{s['name']}"
            if not isinstance(s["typ"], str) or not isinstance(s["nullbar"], bool):
                fehler.append(f"{sn}: typ (Text) und nullbar (true/false) erwartet")
            for k in ("gefuellt", "verschieden", "laenge_min", "laenge_max"):
                if s[k] is not None and (not _ist_int(s[k]) or s[k] < 0):
                    fehler.append(f"{sn}.{k} muss Zahl >= 0 oder null sein")
            if s["formen"] is not None:
                if not isinstance(s["formen"], dict) or set(s["formen"]) != set(FORMEN):
                    fehler.append(f"{sn}.formen braucht genau {', '.join(FORMEN)}")
                elif not all(_ist_int(v) and v >= 0 for v in s["formen"].values()):
                    fehler.append(f"{sn}.formen: nur Zahlen >= 0")
                elif s["gefuellt"] is not None and any(v > s["gefuellt"] for v in s["formen"].values()):
                    fehler.append(f"{sn}.formen: Zähler größer als gefuellt")
            if s["jahre"] is not None:
                if not isinstance(s["jahre"], dict) or not all(re.fullmatch(r"\d{4}", k) and _ist_int(v) for k, v in s["jahre"].items()):
                    fehler.append(f"{sn}.jahre: Objekt Jahr (JJJJ) -> Anzahl erwartet")
            if s["codewerte"] is not None:
                cw = s["codewerte"]
                if not isinstance(cw, list) or not all(isinstance(e, dict) and set(e) == {"wert", "anzahl"} and _ist_int(e["anzahl"])
                                                       and (e["wert"] is None or isinstance(e["wert"], str)) for e in cw):
                    fehler.append(f"{sn}.codewerte: Liste von {{wert, anzahl}} erwartet")
                else:
                    # Datenschutz: dieselben Grenzen wie im Erkundungsskript
                    echte = [e for e in cw if e["wert"] not in (None, "")]
                    if len(echte) > 20:
                        fehler.append(f"{sn}.codewerte: mehr als 20 Werte (Grenze des Erkundungsskripts)")
                    if any(len(e["wert"]) > 3 and e["wert"].upper() not in ("TRUE", "FALSE") for e in echte):
                        fehler.append(f"{sn}.codewerte: Wert länger als 3 Zeichen")
                    if PERSONEN_MUSTER.search(s["name"]) or VERWEIS_MUSTER.search(s["name"]):
                        fehler.append(f"{sn}.codewerte: Spaltenname sieht nach Personen-, Freitext- oder Verweisfeld aus")
                    if s["verschieden"] is not None and s["gefuellt"] is not None and s["verschieden"] >= s["gefuellt"] > 0:
                        fehler.append(f"{sn}.codewerte: alle Werte verschieden (Kennung, kein Code)")
                    hinweise.append(f"{sn}: enthält Codewerte ({len(cw)} Einträge) - vor dem Commit ansehen")
            if basis is not None and s["gefuellt"] is not None and s["gefuellt"] > basis:
                fehler.append(f"{sn}: gefuellt ({s['gefuellt']}) > Zeilen ({basis})")
            if s["gefuellt"] is not None and s["verschieden"] is not None and s["verschieden"] > s["gefuellt"]:
                fehler.append(f"{sn}: verschieden > gefuellt")
            if s["laenge_min"] is not None and s["laenge_max"] is not None and s["laenge_min"] > s["laenge_max"]:
                fehler.append(f"{sn}: laenge_min > laenge_max")
    for k, felder in (("generatoren", {"name", "wert"}), ("trigger", {"name", "tabelle", "typ", "aktiv"}), ("prozeduren", {"name"})):
        if not isinstance(p[k], list) or not all(isinstance(e, dict) and set(e) == felder for e in p[k]):
            fehler.append(f"{k}: Liste von Objekten mit {', '.join(sorted(felder))} erwartet")
    return fehler, hinweise


def profil_laden(pfad):
    with open(pfad, encoding="utf-8-sig") as f:
        p = json.load(f)
    fehler, hinweise = profil_pruefen(p)
    return p, fehler, hinweise


# ---------------------------------------------------------------------------
# Schema (isql -x) lesen: Schlüssel, CHECK-Listen, berechnete Spalten
# ---------------------------------------------------------------------------

def _name(s):
    s = s.strip()
    if s.startswith('"') and s.endswith('"'):
        return s[1:-1].replace('""', '"')
    return s.upper()


def _namensliste(s):
    return [_name(x) for x in s.split(",") if x.strip()]


def _werteliste(s):
    werte = []
    for m in re.finditer(r"'((?:[^']|'')*)'|(-?\d+(?:\.\d+)?)", s):
        werte.append(m.group(1).replace("''", "'") if m.group(1) is not None else m.group(2))
    return werte


ID = r'(?:"(?:[^"]|"")+"|[A-Za-z][\w$]*)'


def schema_lesen(text):
    """Liefert dict mit eindeutig[(tab)] -> [spalten-listen], fks[(tab, spalte)] -> (zieltab, zielspalte),
    checks[(tab, spalte)] -> werte, berechnet {(tab, spalte)}, binaer {(tab, spalte)}."""
    ohne_kommentar = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    erg = {"eindeutig": defaultdict(list), "fks": {}, "checks": {}, "berechnet": set(), "binaer": set()}
    domaenen_check, domaenen_binaer = {}, set()
    for m in re.finditer(rf"CREATE\s+DOMAIN\s+({ID})\s+(?:AS\s+)?(.*?);", ohne_kommentar, re.S | re.I):
        d = _name(m.group(1))
        c = re.search(r"CHECK\s*\(\s*VALUE\s+IN\s*\((.*?)\)\s*\)", m.group(2), re.S | re.I)
        if c:
            domaenen_check[d] = _werteliste(c.group(1))
        if re.search(r"CHARACTER\s+SET\s+OCTETS", m.group(2), re.I):
            domaenen_binaer.add(d)
    for m in re.finditer(rf"ALTER\s+DOMAIN\s+({ID})\s+ADD\s+(?:CONSTRAINT\s+)?CHECK\s*\(\s*VALUE\s+IN\s*\((.*?)\)\s*\)", ohne_kommentar, re.S | re.I):
        domaenen_check[_name(m.group(1))] = _werteliste(m.group(2))

    for m in re.finditer(rf"CREATE\s+TABLE\s+({ID})\s*\((.*?)\);", ohne_kommentar, re.S | re.I):
        tab = _name(m.group(1))
        rumpf = m.group(2)
        # auf oberster Ebene an Kommas trennen
        teile, tiefe, akt, in_text = [], 0, [], False
        for ch in rumpf:
            if ch == "'":
                in_text = not in_text
            if not in_text:
                if ch == "(":
                    tiefe += 1
                elif ch == ")":
                    tiefe -= 1
                elif ch == "," and tiefe == 0:
                    teile.append("".join(akt))
                    akt = []
                    continue
            akt.append(ch)
        teile.append("".join(akt))
        for t in teile:
            t = t.strip()
            if re.match(r"(CONSTRAINT|PRIMARY|UNIQUE|FOREIGN|CHECK)\b", t, re.I):
                _constraint(erg, tab, t)
                continue
            cm = re.match(rf"({ID})\s+(.*)$", t, re.S)
            if not cm:
                continue
            sp, rest = _name(cm.group(1)), cm.group(2)
            # Firebird 2.5: "SP COMPUTED BY (...)", ab 3.0: "SP TYP COMPUTED BY (NULL)" + spaeteres ALTER
            if re.search(r"\bCOMPUTED\s+BY\b|\bGENERATED\s+ALWAYS\s+AS\s*\(|\bGENERATED\s+ALWAYS\s+AS\s+IDENTITY", rest, re.I):
                erg["berechnet"].add((tab, sp))
            typ = re.match(rf"({ID})", rest)
            if typ and _name(typ.group(1)) in domaenen_check:
                erg["checks"][(tab, sp)] = domaenen_check[_name(typ.group(1))]
            if typ and _name(typ.group(1)) in domaenen_binaer or re.search(r"CHARACTER\s+SET\s+OCTETS", rest, re.I):
                erg["binaer"].add((tab, sp))
            c = re.search(rf"CHECK\s*\(\s*({ID})\s+IN\s*\((.*?)\)\s*\)", rest, re.S | re.I)
            if c:
                erg["checks"][(tab, sp)] = _werteliste(c.group(2))
            if re.search(r"\bPRIMARY\s+KEY\b|\bUNIQUE\b", rest, re.I):
                erg["eindeutig"][tab].append([sp])
            r = re.search(rf"REFERENCES\s+({ID})\s*\(\s*({ID})\s*\)", rest, re.I)
            if r:
                erg["fks"][(tab, sp)] = (_name(r.group(1)), _name(r.group(2)))
    for m in re.finditer(rf"ALTER\s+TABLE\s+({ID})\s+ADD\s+(.*?);", ohne_kommentar, re.S | re.I):
        _constraint(erg, _name(m.group(1)), m.group(2))
    for m in re.finditer(rf"CREATE\s+UNIQUE\s+(?:ASC\w*\s+|DESC\w*\s+)?INDEX\s+{ID}\s+ON\s+({ID})\s*\((.*?)\)", ohne_kommentar, re.S | re.I):
        erg["eindeutig"][_name(m.group(1))].append(_namensliste(m.group(2)))
    return erg


def _constraint(erg, tab, t):
    t = re.sub(rf"^CONSTRAINT\s+{ID}\s+", "", t.strip(), flags=re.I)
    m = re.match(r"(PRIMARY\s+KEY|UNIQUE)\s*\((.*?)\)", t, re.S | re.I)
    if m:
        erg["eindeutig"][tab].append(_namensliste(m.group(2)))
        return
    m = re.match(rf"FOREIGN\s+KEY\s*\((.*?)\)\s*REFERENCES\s+({ID})\s*(?:\((.*?)\))?", t, re.S | re.I)
    if m:
        von = _namensliste(m.group(1))
        nach = _namensliste(m.group(3)) if m.group(3) else von
        for a, b in zip(von, nach):
            erg["fks"][(tab, a)] = (_name(m.group(2)), b)
        return
    m = re.match(rf"CHECK\s*\(\s*({ID})\s+IN\s*\((.*?)\)\s*\)", t, re.S | re.I)
    if m:
        erg["checks"][(tab, _name(m.group(1)))] = _werteliste(m.group(2))


# ---------------------------------------------------------------------------
# Spalten-Modell
# ---------------------------------------------------------------------------

GANZ_GRENZE = {"SMALLINT": 32767, "INTEGER": 2147483647, "BIGINT": 9223372036854775807, "INT128": 10 ** 30}


class Spalte:
    def __init__(self, tabelle, roh, basis):
        self.tabelle = tabelle
        self.name = roh["name"]
        self.typ = roh["typ"].upper().strip()
        self.nullbar = roh["nullbar"]
        self.st = roh
        self.basis = basis
        self.laenge = None
        self.varchar = False
        self.skala = 0
        self.praezision = 0
        self.grenze = None
        self.berechnet = False
        self.eindeutig = False
        self.fk = None
        self.check = None
        self.regel = {}
        t = self.typ
        m = re.match(r"^(VARCHAR|CHAR)\((\d+)\)", t)
        if t.startswith("BLOB"):
            self.art = "blob_text" if ("TEXT" in t or re.search(r"SUB_TYPE 1$", t)) else "blob_bin"
        elif m:
            self.art = "text"
            self.varchar = m.group(1) == "VARCHAR"
            self.laenge = int(m.group(2))
            # Profil ohne Formen, aber mit Kennzahlen: Spalte war binaer (OCTETS)
            if roh["gefuellt"] is not None and roh["formen"] is None:
                self.art = "binaer"
        elif t in GANZ_GRENZE:
            self.art = "ganz"
            self.grenze = GANZ_GRENZE[t]
        elif t.startswith(("NUMERIC", "DECIMAL")):
            self.art = "dezimal"
            m = re.search(r"\((\d+)\s*,\s*(\d+)\)", t)
            self.praezision, self.skala = (int(m.group(1)), int(m.group(2))) if m else (9, 2)
        elif t in ("FLOAT", "DOUBLE PRECISION", "REAL") or t.startswith("DECFLOAT"):
            self.art = "gleit"
        elif t == "DATE":
            self.art = "datum"
        elif t.startswith("TIMESTAMP"):
            self.art = "zeitstempel"
        elif t.startswith("TIME"):
            self.art = "zeit"
        elif t == "BOOLEAN":
            self.art = "bool"
        else:
            self.art = "sonst"

    @property
    def schluessel(self):
        return f"{self.tabelle}.{self.name}"


def lade_modell(profil, schema, regeln):
    tabellen = []
    auslassen = {x.upper() for x in regeln.get("auslassen", [])}
    for t in profil["tabellen"]:
        if t["name"].upper() in auslassen:
            continue
        basis = t["stichprobe"] if t["stichprobe"] is not None else t["zeilen"]
        spalten = [Spalte(t["name"], s, basis) for s in t["spalten"]]
        tabellen.append({"name": t["name"], "zeilen": t["zeilen"], "spalten": spalten})
    namen = {t["name"] for t in tabellen}
    for t in tabellen:
        sp_namen = {s.name for s in t["spalten"]}
        for s in t["spalten"]:
            k = (t["name"], s.name)
            if schema:
                s.berechnet = k in schema["berechnet"]
                s.check = schema["checks"].get(k)
                if k in schema["binaer"] and s.art == "text":
                    s.art = "binaer"
                if k in schema["fks"] and schema["fks"][k][0] in namen:
                    s.fk = schema["fks"][k]
        if schema:
            for liste in schema["eindeutig"].get(t["name"], []):
                liste = [x for x in liste if x in sp_namen]
                if liste:
                    # Eindeutigkeit über die letzte Spalte sicherstellen (reicht auch für zusammengesetzte Schlüssel)
                    for s in t["spalten"]:
                        if s.name == liste[-1]:
                            s.eindeutig = True
    for v in regeln.get("verweise", []):
        von_t, von_s = v["von"].split(".", 1)
        nach_t, nach_s = v["nach"].split(".", 1)
        for t in tabellen:
            for s in t["spalten"]:
                if t["name"] == von_t and s.name == von_s and nach_t in namen:
                    s.fk = (nach_t, nach_s)
    for schl, r in regeln.get("spalten", {}).items():
        tn, sn = schl.split(".", 1)
        for t in tabellen:
            for s in t["spalten"]:
                if t["name"] == tn and s.name == sn:
                    s.regel = r
    return tabellen


def reihenfolge(tabellen):
    """Tabellen so ordnen, dass Verweisziele zuerst gefüllt werden (Zyklen werden aufgebrochen)."""
    nach_name = {t["name"]: t for t in tabellen}
    erledigt, ergebnis, aktiv = set(), [], set()

    def besuche(n):
        if n in erledigt or n in aktiv:
            return
        aktiv.add(n)
        for s in nach_name[n]["spalten"]:
            if s.fk and s.fk[0] != n and s.fk[0] in nach_name:
                besuche(s.fk[0])
        aktiv.discard(n)
        erledigt.add(n)
        ergebnis.append(nach_name[n])

    for t in tabellen:
        besuche(t["name"])
    return ergebnis


# ---------------------------------------------------------------------------
# Werte erzeugen
# ---------------------------------------------------------------------------

def zuteilen(gewichte, n):
    """Ganzzahlige Aufteilung von n nach Gewichten (größter Rest)."""
    summe = sum(g for _, g in gewichte)
    if summe <= 0 or n <= 0:
        return [(w, 0) for w, _ in gewichte]
    roh = [(w, g * n / summe) for w, g in gewichte]
    ganz = [(w, int(x)) for w, x in roh]
    rest = n - sum(x for _, x in ganz)
    reihen = sorted(range(len(roh)), key=lambda i: roh[i][1] - int(roh[i][1]), reverse=True)
    ganz = [list(x) for x in ganz]
    for i in reihen[:rest]:
        ganz[i][1] += 1
    return [tuple(x) for x in ganz]


class Erzeuger:
    def __init__(self, seed, heute=None):
        self.rnd = random.Random(seed)
        self.heute = heute or dt.date(2026, 11, 20)

    # -- Hilfen --------------------------------------------------------------
    def jahr(self, sp):
        jahre = sp.st.get("jahre")
        if jahre:
            gewichte = [(int(j), n) for j, n in jahre.items() if n > 0]
            if gewichte:
                return self.rnd.choices([j for j, _ in gewichte], weights=[n for _, n in gewichte])[0]
        if re.search(r"GEB", sp.name, re.I):
            return self.rnd.randint(1935, 2018)
        return self.rnd.randint(self.heute.year - 6, self.heute.year)

    def datum(self, sp):
        j = self.jahr(sp)
        tag = self.rnd.randint(0, 364)
        return dt.date(j, 1, 1) + dt.timedelta(days=tag)

    def woerter(self, laenge, ascii_nur=False):
        quelle = WOERTER_ASCII if ascii_nur else WOERTER
        teile = []
        while len(" ".join(teile)) < laenge:
            teile.append(self.rnd.choice(quelle))
        return " ".join(teile)[:laenge].rstrip() or "x"

    def laenge(self, sp, minimal=1):
        lo = sp.st.get("laenge_min") or minimal
        hi = sp.st.get("laenge_max") or max(lo, min(sp.laenge or 20, 20))
        if sp.laenge:
            hi = min(hi, sp.laenge)
            lo = min(lo, hi)
        return self.rnd.randint(max(1, lo), max(1, hi))

    # -- Textwerte -----------------------------------------------------------
    def textform(self, sp):
        f = sp.st.get("formen")
        g = sp.st.get("gefuellt") or 0
        if not f or g <= 0:
            return "text"
        d1, d2, geld = f["datum_jjjjmmtt"], f["datum_ttmmjjjj"], f["geld"]
        gewichte = {
            "datum_jjjjmmtt": d1,
            "datum_ttmmjjjj": d2,
            "geld": geld,
            "ziffern": max(0, f["ziffern"] - d1 - (d2 if (sp.st.get("laenge_max") or 0) <= 8 else 0)),
            "text": max(0, f["buchstaben"] - geld),
        }
        gewichte["rest"] = max(0, g - sum(gewichte.values()))
        formen = [k for k, v in gewichte.items() if v > 0] or ["text"]
        return self.rnd.choices(formen, weights=[gewichte[k] for k in formen] if len(formen) > 1 else None)[0]

    def semantischer_text(self, sp):
        art = sp.regel.get("art", "")
        n = sp.name.upper()
        if art == "vorname" or (not art and "VORNAME" in n):
            if self.rnd.random() < 0.15:
                return "-".join(self.rnd.sample(VORNAMEN, 2))
            return self.rnd.choice(VORNAMEN)
        if art == "nachname" or (not art and "NAME" in n):
            return self.rnd.choice(NACHNAMEN)
        if art == "strasse" or (not art and re.search(r"STRASSE|STRAS|^STR", n)):
            return f"{self.rnd.choice(STRASSEN)} {self.rnd.randint(1, 120)}"
        if art == "ort" or (not art and re.search(r"^ORT|ORT$|WOHNORT", n)):
            return self.rnd.choice(ORTE)
        if art == "email" or (not art and "MAIL" in n):
            return f"{self.rnd.choice(VORNAMEN).lower()}.{self.rnd.choice(NACHNAMEN).lower()}@beispiel.invalid".replace("ü", "ue").replace("ß", "ss")
        return None

    def textwert(self, sp, form, nummer=None):
        lmax_dekl = sp.laenge or 50
        if form == "datum_jjjjmmtt":
            return self.datum(sp).strftime("%Y%m%d")
        if form == "datum_ttmmjjjj":
            punkte = (sp.st.get("laenge_min") or 8) >= 10
            return self.datum(sp).strftime("%d.%m.%Y" if punkte else "%d%m%Y")
        if form == "geld":
            l = max(3, min(self.laenge(sp, 3), lmax_dekl))
            cent = str(self.rnd.randint(0, 10 ** min(l - 1, 7) - 1))
            return "e" + cent.rjust(l - 1)
        if form == "ziffern":
            l = self.laenge(sp)
            if nummer is not None:
                # laufende Nummer; nur bei fester Breite mit Nullen auffüllen
                fest = sp.st.get("laenge_min") if sp.st.get("laenge_min") == sp.st.get("laenge_max") else 1
                return str(nummer).zfill(fest or 1)
            return "".join(self.rnd.choice(string.digits) for _ in range(l))
        if form == "rest":
            l = self.laenge(sp)
            if l == 1:
                return self.rnd.choice("-./+")
            return ("".join(self.rnd.choice(string.digits) for _ in range(l - 1)) + "-")[:l]
        # Text mit Buchstaben
        sem = self.semantischer_text(sp)
        lo = sp.st.get("laenge_min") or 1
        hi = min(sp.st.get("laenge_max") or lmax_dekl, lmax_dekl)
        if sem is None:
            l = self.laenge(sp)
            if l <= 3:
                sem = "".join(self.rnd.choice(string.ascii_uppercase) for _ in range(l))
            else:
                sem = self.woerter(l)
        while len(sem) < lo:
            sem += " " + self.rnd.choice(NACHNAMEN)
        return sem[:hi].rstrip() if len(sem) > hi else sem

    def auffuellen(self, sp, wert):
        """Rechts-/linksbündig auffüllen wie im Profil (je verschiedenem Wert entschieden)."""
        f = sp.st.get("formen")
        g = sp.st.get("gefuellt") or 0
        if not f or g <= 0 or not sp.laenge:
            return wert
        r = f["rechtsbuendig"] / g
        l = f["linksbuendig"] / g if sp.varchar else 0
        z = self.rnd.random()
        if z < r:
            if len(wert) >= sp.laenge:
                wert = wert[: sp.laenge - 1]
            return wert.rjust(sp.laenge)
        if z < r + l and len(wert) < sp.laenge:
            return wert.ljust(sp.laenge)
        return wert

    # -- ein Pool verschiedener Werte -----------------------------------------
    def pool(self, sp, k):
        werte, gesehen = [], set()
        versuche = 0
        while len(werte) < k and versuche < k * 30 + 100:
            versuche += 1
            nummer = len(werte) + 1 if sp.eindeutig else None
            w = self.einzelwert(sp, nummer)
            schl = w.strip(" ") if isinstance(w, str) else w
            if schl in gesehen or schl == "":
                continue
            gesehen.add(schl)
            werte.append(w)
        while len(werte) < k:  # Wertebereich erschöpft: mit Zähler eindeutig machen
            w = self.einzelwert(sp, len(werte) + 1)
            if isinstance(w, str):
                w = (w.strip() + str(len(werte)))[-(sp.laenge or 50):]
            elif isinstance(w, int) and not isinstance(w, bool):
                w = len(werte) + 1
            schl = w.strip(" ") if isinstance(w, str) else w
            if schl in gesehen:
                break
            gesehen.add(schl)
            werte.append(w)
        return werte

    def einzelwert(self, sp, nummer=None):
        a = sp.art
        if a == "text":
            if sp.eindeutig and nummer is not None:
                f = sp.st.get("formen") or {}
                g = sp.st.get("gefuellt") or 0
                if g and f.get("ziffern", 0) >= 0.5 * g:
                    return self.auffuellen(sp, self.textwert(sp, "ziffern", nummer))
            return self.auffuellen(sp, self.textwert(sp, self.textform(sp)))
        if a == "ganz":
            if sp.eindeutig and nummer is not None:
                return nummer
            obergrenze = min(sp.grenze, 10 ** max(2, len(str(max(1, sp.st.get("verschieden") or 10))) + 1))
            return self.rnd.randint(0, obergrenze)
        if a == "dezimal":
            vor = min(10 ** max(1, sp.praezision - sp.skala) - 1, 2000)
            return Decimal(self.rnd.randint(0, vor * 10 ** sp.skala)) / (10 ** sp.skala)
        if a == "gleit":
            return round(self.rnd.uniform(0, 1000), 4)
        if a == "datum":
            return self.datum(sp)
        if a == "zeitstempel":
            d = self.datum(sp)
            return dt.datetime(d.year, d.month, d.day, self.rnd.randint(7, 19), self.rnd.randint(0, 59), self.rnd.randint(0, 59))
        if a == "zeit":
            return dt.time(self.rnd.randint(7, 19), self.rnd.choice(range(0, 60, 5)))
        if a == "bool":
            return self.rnd.random() < 0.5
        if a == "blob_text":
            lo = sp.st.get("laenge_min") or 20
            hi = min(sp.st.get("laenge_max") or 200, 2000)
            return self.woerter(self.rnd.randint(min(lo, hi), hi), ascii_nur=True)
        if a == "blob_bin":
            lo = sp.st.get("laenge_min") or 16
            hi = min(sp.st.get("laenge_max") or 64, 256)
            return bytes(self.rnd.getrandbits(8) for _ in range(self.rnd.randint(min(lo, hi), hi)))
        if a == "binaer":
            return bytes(self.rnd.getrandbits(8) for _ in range(sp.laenge or 16))
        return None

    # -- eine Spalte für n Zeilen --------------------------------------------
    def spalte(self, sp, n, vorhanden):
        """Liefert Liste mit n Werten (None = NULL)."""
        if n == 0:
            return []
        # 1. feste Verteilung: Regel > Codewerte > CHECK-Liste
        verteilung = None
        if "werte" in sp.regel:
            verteilung = [(w, g) for w, g in sp.regel["werte"].items()]
        elif sp.st.get("codewerte"):
            verteilung = [(e["wert"], e["anzahl"]) for e in sp.st["codewerte"]]
        elif sp.check and not sp.eindeutig:
            verteilung = [(w, 1) for w in sp.check]
        if verteilung:
            werte = []
            for w, anzahl in zuteilen(verteilung, n):
                werte += [self.umwandeln(sp, w)] * anzahl
            self.rnd.shuffle(werte)
            if not sp.nullbar:
                werte = [self.leer(sp) if w is None else w for w in werte]
            return werte

        # 2. Füllgrad
        g = sp.st.get("gefuellt")
        if sp.eindeutig or sp.regel.get("art") == "laufend":
            anteil = 1.0
        elif g is not None and sp.basis:
            anteil = g / sp.basis
        elif sp.basis == 0:
            anteil = 0.0
        else:
            anteil = 1.0 if not sp.nullbar else 0.8
        if sp.art in ("sonst",):
            anteil = 0.0
        k_fill = int(round(anteil * n))
        if not sp.nullbar and sp.art not in ("text", "blob_text", "blob_bin"):
            k_fill = n

        # 3. Anzahl verschiedener Werte
        v = sp.st.get("verschieden")
        if sp.eindeutig or sp.regel.get("art") == "laufend":
            k_dist = k_fill
        elif v is not None and g:
            k_dist = k_fill if v >= g else max(1, int(round(v / g * k_fill)))
        else:
            k_dist = max(1, k_fill // 2)
        k_dist = min(k_dist, k_fill)

        # 4. Pool: Verweisziel oder erzeugt
        if sp.regel.get("art") == "laufend":
            breite = sp.regel.get("breite") or sp.laenge
            pool = [i + 1 for i in range(k_dist)]
            if sp.art == "text":
                pool = [str(i).rjust(breite) if sp.regel.get("rechtsbuendig") else str(i) for i in pool]
        elif sp.fk and vorhanden.get(sp.fk):
            ziel = list(dict.fromkeys(x for x in vorhanden[sp.fk] if x is not None))
            self.rnd.shuffle(ziel)
            pool = ziel[:k_dist] if not sp.eindeutig else ziel
            k_fill = min(k_fill, len(pool)) if sp.eindeutig else k_fill
        elif sp.regel.get("art") and sp.art == "text":
            pool, gesehen = [], set()
            for _ in range(k_dist * 20):
                w = self.semantischer_text(sp) or self.textwert(sp, "text")
                if w not in gesehen:
                    gesehen.add(w)
                    pool.append(w[: sp.laenge] if sp.laenge else w)
                if len(pool) >= k_dist:
                    break
        else:
            pool = self.pool(sp, k_dist)
        if not pool:
            k_fill = 0

        # 5. Zuordnen: jeder Poolwert mindestens einmal, Rest leicht schief verteilt
        gefuellt = list(pool[:k_fill])
        if len(gefuellt) < k_fill:
            gew = [1.0 / (i + 1) ** 0.6 for i in range(len(pool))]
            gefuellt += self.rnd.choices(pool, weights=gew, k=k_fill - len(gefuellt))
        self.rnd.shuffle(gefuellt)
        leer = [self.leer(sp) if not sp.nullbar else None] * (n - k_fill)
        werte = gefuellt + leer
        if not (sp.eindeutig and k_fill == n):
            self.rnd.shuffle(werte)
        return werte

    def leer(self, sp):
        if sp.art == "text":
            return ""
        if sp.art in ("blob_text",):
            return ""
        if sp.art in ("blob_bin", "binaer"):
            return b""
        if sp.art in ("ganz", "dezimal", "gleit"):
            return 0
        if sp.art == "bool":
            return False
        return self.einzelwert(sp)

    def umwandeln(self, sp, w):
        """Wert aus Codewerten/Regeln (Text) in den Spaltentyp."""
        if w is None or w == "<null>":
            return None
        if sp.art == "ganz":
            try:
                return int(w)
            except ValueError:
                return 0
        if sp.art == "dezimal":
            return Decimal(str(w).replace(",", "."))
        if sp.art == "gleit":
            return float(str(w).replace(",", "."))
        if sp.art == "bool":
            return str(w).strip().upper() in ("TRUE", "1", "J", "<TRUE>")
        return str(w)


def erzeugen(tabellen, args, regeln, erzeuger=None, ascii_nur=False):
    """Erzeugt Zeilen je Tabelle. Rückgabe: Liste (tabelle, spalten, zeilen)."""
    erz = erzeuger or Erzeuger(args.seed)
    vorhanden = {}
    ergebnis = []
    for t in reihenfolge(tabellen):
        z = regeln.get("zeilen", {}).get(t["name"])
        if z is None:
            if t["zeilen"] is None:
                z = args.standard_zeilen
            elif t["zeilen"] == 0:
                z = 0
            else:
                z = max(args.min_zeilen, int(round(t["zeilen"] * args.skala)))
                z = max(1, min(z, args.max_zeilen))
        spalten = [s for s in t["spalten"] if not s.berechnet and s.art != "sonst" and s.regel.get("art") != "auslassen"]
        werte = {}
        for s in spalten:
            werte[s.name] = erz.spalte(s, z, vorhanden)
            if ascii_nur:
                werte[s.name] = [w.translate(ERSATZ).encode("ascii", "replace").decode("ascii") if isinstance(w, str) else w
                                 for w in werte[s.name]]
            vorhanden[(t["name"], s.name)] = werte[s.name]
        zeilen = [[werte[s.name][i] for s in spalten] for i in range(z)]
        ergebnis.append((t["name"], spalten, zeilen))
    return ergebnis


# ---------------------------------------------------------------------------
# Kennzahlen wie in S1-erkundung.ps1 (für --bericht und Tests)
# ---------------------------------------------------------------------------

RE_JJJJMMTT = re.compile(r"(19|20)[0-9]{2}(0[1-9]|1[0-2])(0[1-9]|[12][0-9]|3[01])")
RE_TTMMJJJJ = re.compile(r"(0[1-9]|[12][0-9]|3[01])[.]?(0[1-9]|1[0-2])[.]?(19|20)[0-9]{2}")
RE_GELD1 = re.compile(r"[A-Za-z] *-?[0-9]+")
RE_GELD2 = re.compile(r"-?[0-9]+[.,][0-9]{2}")


def kennzahlen(sp, werte):
    """Kennzahlen einer Spalte nach denselben Regeln wie die Profil-SQL des Erkundungsskripts."""
    k = {"gefuellt": None, "verschieden": None, "laenge_min": None, "laenge_max": None, "formen": None, "jahre": None}
    if sp.art == "text":
        roh = [w if (w is None or not sp.laenge or sp.varchar) else w.ljust(sp.laenge) for w in werte]
        e = [None if w is None or w.strip(" ") == "" else w.strip(" ") for w in roh]
        voll = [(r, x) for r, x in zip(roh, e) if x is not None]
        k["gefuellt"] = len(voll)
        k["verschieden"] = len({x for _, x in voll})
        if voll:
            k["laenge_min"] = min(len(x) for _, x in voll)
            k["laenge_max"] = max(len(x) for _, x in voll)
        f = dict.fromkeys(FORMEN, 0)
        for r, x in voll:
            f["ziffern"] += bool(re.fullmatch(r"[0-9]+", x))
            f["datum_jjjjmmtt"] += len(x) == 8 and bool(RE_JJJJMMTT.fullmatch(x))
            f["datum_ttmmjjjj"] += len(x) in (8, 10) and bool(RE_TTMMJJJJ.fullmatch(x))
            f["rechtsbuendig"] += len(r.lstrip(" ")) < len(r)
            f["linksbuendig"] += len(r.rstrip(" ")) < len(r)
            f["buchstaben"] += bool(re.search(r"[A-Za-z]", x))
            f["geld"] += bool(RE_GELD1.fullmatch(x) or RE_GELD2.fullmatch(x))
        k["formen"] = f
        g = k["gefuellt"]
        if g:
            if f["datum_jjjjmmtt"] >= 0.8 * g:
                j = defaultdict(int)
                for _, x in voll:
                    if len(x) == 8 and RE_JJJJMMTT.fullmatch(x):
                        j[x[:4]] += 1
                k["jahre"] = dict(sorted(j.items()))
            elif f["datum_ttmmjjjj"] >= 0.8 * g:
                j = defaultdict(int)
                for _, x in voll:
                    if len(x) in (8, 10) and RE_TTMMJJJJ.fullmatch(x):
                        j[x[-4:]] += 1
                k["jahre"] = dict(sorted(j.items()))
    elif sp.art in ("blob_text", "blob_bin"):
        laengen = [len(w.encode("latin-1", "replace")) if isinstance(w, str) else len(w) for w in werte if w is not None]
        laengen = [x for x in laengen if x > 0]
        k["gefuellt"] = len(laengen)
        if laengen:
            k["laenge_min"], k["laenge_max"] = min(laengen), max(laengen)
    else:
        voll = [w for w in werte if w is not None]
        k["gefuellt"] = len(voll)
        k["verschieden"] = len(set(voll))
        if sp.art in ("datum", "zeitstempel") and voll:
            j = defaultdict(int)
            for w in voll:
                j[str(w.year)] += 1
            k["jahre"] = dict(sorted(j.items()))
    return k


# ---------------------------------------------------------------------------
# SQL
# ---------------------------------------------------------------------------

def q(name, dialekt):
    if dialekt == 1:
        return name
    return '"' + name.replace('"', '""') + '"'


def literal(sp, w):
    if w is None:
        return "NULL"
    if sp.art in ("blob_bin", "binaer"):
        return "x'" + w.hex() + "'" if w else "''"
    if sp.art == "bool":
        return "TRUE" if w else "FALSE"
    if sp.art in ("ganz", "gleit") or sp.art == "dezimal":
        return str(w)
    if sp.art == "datum":
        return f"'{w.isoformat()}'"
    if sp.art == "zeitstempel":
        return f"'{w.strftime('%Y-%m-%d %H:%M:%S')}'"
    if sp.art == "zeit":
        return f"'{w.strftime('%H:%M:%S')}'"
    return "'" + str(w).replace("'", "''") + "'"


def anweisungen(profil, daten, args, trigger_aus):
    dialekt = profil["firebird"].get("dialekt") or 3
    out = []
    gefuellte = {t for t, _, z in daten if z}
    trigger = [t for t in profil["trigger"] if t["aktiv"] and t["tabelle"] in gefuellte] if trigger_aus else []
    for t in trigger:
        out.append(f"ALTER TRIGGER {q(t['name'], dialekt)} INACTIVE;")
    if trigger:
        out.append("COMMIT;")
    groesste_id = 0
    for tab, spalten, zeilen in daten:
        if not zeilen:
            continue
        kopf = f"INSERT INTO {q(tab, dialekt)} ({', '.join(q(s.name, dialekt) for s in spalten)}) VALUES ("
        for i, z in enumerate(zeilen, 1):
            out.append(kopf + ", ".join(literal(s, w) for s, w in zip(spalten, z)) + ");")
            if i % 1000 == 0:
                out.append("COMMIT;")
        out.append("COMMIT;")
        for s, spalte_werte in zip(spalten, zip(*zeilen)):
            if s.eindeutig and s.art == "ganz":
                groesste_id = max([groesste_id] + [w for w in spalte_werte if isinstance(w, int)])
    for t in trigger:
        out.append(f"ALTER TRIGGER {q(t['name'], dialekt)} ACTIVE;")
    if trigger:
        out.append("COMMIT;")
    # Generatoren auf den Stand aus dem Profil (mindestens größte erzeugte ID), damit neue Zeilen nicht kollidieren
    for g in profil["generatoren"]:
        wert = max(g["wert"] or 0, groesste_id)
        out.append(f"SET GENERATOR {q(g['name'], dialekt)} TO {wert};")
    if profil["generatoren"]:
        out.append("COMMIT;")
    return out


def nur_ascii(profil):
    """Bei Zeichensatz NONE speichert Firebird Bytes ohne Umwandlung; THEORG schreibt dann vermutlich
    WIN1252-Bytes, eine UTF8-Verbindung dagegen UTF-8-Bytes. Dann nur ASCII erzeugen (ä -> ae)."""
    return (profil["firebird"].get("zeichensatz") or "NONE").upper() in ("NONE", "OCTETS", "ASCII")


ERSATZ = str.maketrans({"ä": "ae", "ö": "oe", "ü": "ue", "Ä": "Ae", "Ö": "Oe", "Ü": "Ue", "ß": "ss"})


def direkt_schreiben(dsn, benutzer, stmts, cs, fbclient=None):
    try:
        from firebird.driver import connect, driver_config  # type: ignore
    except ImportError:
        raise Fehler("--direkt braucht das Paket firebird-driver (pip install firebird-driver) und fbclient")
    if fbclient:
        driver_config.fb_client_library.value = fbclient
    passwort = os.environ.get("ISC_PASSWORD")
    if not passwort:
        raise Fehler("--direkt: Passwort in der Umgebungsvariablen ISC_PASSWORD angeben")
    con = connect(dsn, user=benutzer, password=passwort, charset=cs)

    def festschreiben():
        if con.main_transaction.is_active():
            con.commit()

    try:
        cur = con.cursor()
        for s in stmts:
            if s == "COMMIT;":
                festschreiben()
                continue
            cur.execute(s.rstrip(";"))
        festschreiben()
    finally:
        con.close()


# ---------------------------------------------------------------------------
# Bericht Soll/Ist
# ---------------------------------------------------------------------------

def bericht(tabellen, daten):
    nach = {t["name"]: t for t in tabellen}
    zeilen = []
    for tab, spalten, werte in daten:
        t = nach[tab]
        zeilen.append(f"{tab}: {len(werte)} Zeilen (Profil {t['zeilen']})")
        for i, s in enumerate(spalten):
            k = kennzahlen(s, [z[i] for z in werte])
            g_soll = s.st.get("gefuellt")
            if g_soll is None or not s.basis or not werte:
                continue
            soll = g_soll / s.basis
            ist = (k["gefuellt"] or 0) / len(werte)
            if abs(soll - ist) > 0.1:
                zeilen.append(f"  {s.name}: Füllgrad Profil {soll:.0%}, erzeugt {ist:.0%}")
    return "\n".join(zeilen)


# ---------------------------------------------------------------------------

def main(argv=None):
    ap = argparse.ArgumentParser(description="Erfundene Daten für die THEORG-Nachbildung aus S1-profil_*.json")
    ap.add_argument("profil", help="Profil-JSON von S1-erkundung.ps1 (Format 1)")
    ap.add_argument("--schema", help="Schema-SQL (isql -x) für Schlüssel, CHECK-Listen, berechnete Spalten")
    ap.add_argument("--regeln", help="Regeldatei (JSON), siehe regeln.beispiel.json")
    ap.add_argument("--skala", type=float, default=0.05, help="Anteil der Zeilenzahl aus dem Profil (Standard 0.05)")
    ap.add_argument("--max-zeilen", type=int, default=5000, help="höchstens so viele Zeilen je Tabelle (Standard 5000)")
    ap.add_argument("--min-zeilen", type=int, default=3, help="mindestens so viele Zeilen je nicht leerer Tabelle (Standard 3)")
    ap.add_argument("--standard-zeilen", type=int, default=20, help="Zeilen je Tabelle, wenn das Profil keine Zeilenzahl hat")
    ap.add_argument("--seed", type=int, default=1, help="Startwert des Zufallsgenerators (gleiche Eingabe -> gleiche Daten)")
    ap.add_argument("--ausgabe", default="daten.sql", help="SQL-Datei (Standard daten.sql); '-' = Standardausgabe")
    ap.add_argument("--mit-triggern", action="store_true", help="Trigger beim Einspielen aktiv lassen (Standard: vorher abschalten, danach wieder an)")
    ap.add_argument("--direkt", metavar="DSN", help="statt SQL-Datei direkt schreiben, z. B. localhost/3050:/firebird/data/theorg.fdb")
    ap.add_argument("--benutzer", default="SYSDBA", help="Benutzer für --direkt (Passwort aus ISC_PASSWORD)")
    ap.add_argument("--fbclient", help="Pfad zur Firebird-Clientbibliothek für --direkt (libfbclient.so / fbclient.dll)")
    ap.add_argument("--bericht", action="store_true", help="Abweichungen Füllgrad Profil/erzeugt ausgeben")
    ap.add_argument("--nur-pruefen", action="store_true", help="nur Format und Datenschutzgrenzen des Profils prüfen")
    args = ap.parse_args(argv)

    profil, fehler, hinweise = profil_laden(args.profil)
    for h in hinweise:
        print("Hinweis:", h, file=sys.stderr)
    for f in fehler:
        print("Fehler:", f, file=sys.stderr)
    if fehler:
        return 1
    if args.nur_pruefen:
        print(f"{args.profil}: Format 1 in Ordnung ({len(profil['tabellen'])} Tabellen).")
        return 0

    regeln = {}
    if args.regeln:
        with open(args.regeln, encoding="utf-8") as f:
            regeln = json.load(f)
    schema = None
    if args.schema:
        with open(args.schema, encoding="utf-8", errors="replace") as f:
            schema = schema_lesen(f.read())
    else:
        print("Hinweis: ohne --schema keine Schlüssel/CHECK-Listen/berechneten Spalten; Einspielen kann daran scheitern.", file=sys.stderr)

    tabellen = lade_modell(profil, schema, regeln)
    daten = erzeugen(tabellen, args, regeln, ascii_nur=nur_ascii(profil))
    stmts = anweisungen(profil, daten, args, trigger_aus=not args.mit_triggern)
    # Immer UTF8: isql 3 verliert Umlaute aus 8-Bit-Eingaben ohne UTF-8-Locale (nachbilden.sh setzt LANG=C.UTF-8)
    cs_name, kodierung = "UTF8", "utf-8"
    anzahl = sum(len(z) for _, _, z in daten)

    if args.direkt:
        direkt_schreiben(args.direkt, args.benutzer, stmts, cs_name, args.fbclient)
        print(f"{anzahl} Zeilen in {len(daten)} Tabellen direkt geschrieben.", file=sys.stderr)
    else:
        kopf = [
            "-- Erfundene Daten für die THEORG-Nachbildung (theorg/db/fuellen.py). KEINE echten Daten.",
            f"-- Profil: {os.path.basename(args.profil)} vom {profil['erstellt']}; Skala {args.skala}; Seed {args.seed}",
            f"-- isql-zeichensatz: {cs_name}",
            "",
        ]
        text = "\n".join(kopf + stmts) + "\n"
        if args.ausgabe == "-":
            sys.stdout.buffer.write(text.encode(kodierung, "replace"))
        else:
            with open(args.ausgabe, "w", encoding=kodierung, errors="replace", newline="\n") as f:
                f.write(text)
            print(f"{anzahl} Zeilen in {len(daten)} Tabellen -> {args.ausgabe} ({cs_name})", file=sys.stderr)
    if args.bericht:
        print(bericht(tabellen, daten), file=sys.stderr)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Fehler as e:
        print("Fehler:", e, file=sys.stderr)
        sys.exit(2)
