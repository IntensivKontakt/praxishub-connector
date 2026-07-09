# Z1-Abrechnung & Honorar — Referenz für die Praxis-Steuerung

Ergänzt `docs/Z1-DATABASE.md` um das **Abrechnungs-/Honorarmodell**. Am ZMM-Piloten
(09.07.2026) gegen die echte Z1-DB verifiziert. Grundlage für den Control-Sync
(`core/src/z1db/control.rs`). **Nichts hier raten — dieses Dokument ist die Wahrheit.**

## 1. Geldformat (gilt DB-weit)

Beträge sind **keine Dezimalzahlen**, sondern **Währungspräfix `e` + Cent-Ganzzahl**,
rechtsbündig, ohne Trennzeichen: `"e 225992"` = **2.259,92 €**. Wenige Zeilen haben statt
`e` ein Leerzeichen — identisch parsen. Naives Zahl-Parsen (`TRY_CAST … AS float`) → **null**.

```sql
-- Roh → Euro (ALLE Betragsfelder: CASH.BETRAG, FAKT.ZHON, BEH.DMBETRAG, …):
CAST(NULLIF(REPLACE(REPLACE(<feld>,'e',''),' ',''),'') AS bigint) / 100.0
```

**Weitere Fixkomma-Skalierungen (DB-weit):**
- `ANZAHL` ×100 (`"100"` = 1,00)
- `FAKTOR` ×10000 (`"35000"` = 3,5)
- `GO.GEBPKT` ×100 (Punkte)
- `PUNKTWERTE.PWWEST` ÷10000 (€/Punkt)

**Stornos sind nie negativ** — Flag `STORNIERT='1'` markiert die Umkehr (ausschließen oder
negieren), es gibt **kein Minus im Feld**.

## 2. Tabellen

| Zweck | Tabelle | Schlüsselspalten |
|---|---|---|
| Zahlungen (bezahlt) | `CASH` | `BETRAG` (100 %), `STORNIERT`, `ZAHLUNGSWEG`, `CASHDATUM` |
| Rechnungen (fakturiert) | `FAKT` | `BETRAG`, `ZHON` (Honorar), `ELAB`/`FLAB` (Eigen-/Fremdlabor), `KTRAEGER` (1=GKV, z/a=privat), `STORNIERT`, `FAKTDATUM` |
| Abrechnungsfall | `BILL` | `BETRAG`, `VONBEHDATUM`/`BISBEHDATUM`, `LFDHPLAN` |
| Leistungen (pro Zeile) | `BEH` (1,46 Mio) | `LEBID`, `GOART`, `LSTNR`/`KYLSTNR`, `ANZAHL`, `DMBETRAG` |
| Laborposten | `LBLOCKENTRY` | `EINZELBETRAG` (91 %) |
| Gebührenkatalog | `GO` (12,3k) | `KYLSTNR`, `GEBPKT` (Punkte), `EINFACHSATZ` (GOZ €), `PWLART`, `ABDATUM`/`BISDATUM` |
| Punktwerte | `PUNKTWERTE` | `PWLART`, `PWGROUP` (Kasse), `PWWEST`, `ABDATUM` |
| Behandler | `LEB` → `PERSONAL` | `LEB.LEBID`→`LEB.PID`→`PERSONAL.PID`, Name = `PERSONAL.KUERZEL` |

## 3. Die zwei Reporting-Achsen

- **Behandler** = `BEH.LEBID` (~100 % befüllt) → `LEB.PID` → `PERSONAL.KUERZEL`.
  (`LEB.BEZEICHNUNG` ist leer — **nicht** benutzen.)
- **BEMA vs GOZ** = `BEH.GOART`: `g` = BEMA/GKV · `q` = GOZ (Haupt-Privathonorar) ·
  `2/3/4/7` = Material/Sonderpositionen (privat) · leer = Befund/kein Honorar.

## 4. Euro je Segment

**GOZ + Privat (`q,2,3,4,7`)** → steht in `BEH.DMBETRAG`, direkt summierbar (exakt):
```sql
SELECT p.KUERZEL, b.GOART, COUNT(*) AS leistungen,
       SUM(CAST(NULLIF(REPLACE(REPLACE(b.DMBETRAG,'e',''),' ',''),'') AS bigint))/100.0 AS goz_euro
FROM BEH b
LEFT JOIN LEB l      ON LTRIM(RTRIM(l.LEBID)) = LTRIM(RTRIM(b.LEBID))
LEFT JOIN PERSONAL p ON LTRIM(RTRIM(p.PID))   = LTRIM(RTRIM(l.PID))
WHERE b.DATUM >= '20260101' AND b.GOART <> 'g' AND LTRIM(RTRIM(b.GOART)) <> ''
GROUP BY p.KUERZEL, b.GOART;
```

**BEMA (`g`)** → nicht in `BEH`, aber aus `GO` berechenbar (`BEH.KYLSTNR = GO.KYLSTNR`, 100 % Match):
Honorar = `GEBPKT/100 × ANZAHL/100 × Punktwert(PWLART)`, Punktwert = `PWWEST/10000`.
```sql
;WITH pw AS (   -- aktueller Punktwert je PWLART (über Kassen gemittelt)
  SELECT PWLART, AVG(CAST(NULLIF(REPLACE(REPLACE(PWWEST,'e',''),' ',''),'') AS float))/10000.0 AS pwert
  FROM PUNKTWERTE p
  WHERE ABDATUM=(SELECT MAX(ABDATUM) FROM PUNKTWERTE p2 WHERE p2.PWLART=p.PWLART AND p2.ABDATUM<='20260709')
    AND LTRIM(RTRIM(REPLACE(PWWEST,'e','')))<>'' GROUP BY PWLART )
SELECT p.KUERZEL, COUNT(*) AS zeilen,
       SUM(CAST(NULLIF(REPLACE(g.GEBPKT,' ',''),'') AS float)/100.0
           * CAST(NULLIF(REPLACE(b.ANZAHL,' ',''),'') AS float)/100.0
           * ISNULL(pw.pwert,0)) AS bema_euro
FROM BEH b
CROSS APPLY (SELECT TOP 1 GEBPKT, PWLART FROM GO
             WHERE KYLSTNR=b.KYLSTNR AND ABDATUM<=b.DATUM
               AND (BISDATUM IS NULL OR LTRIM(RTRIM(BISDATUM))='' OR BISDATUM>=b.DATUM)
               AND LTRIM(RTRIM(GEBPKT))<>'' ORDER BY ABDATUM DESC) g
LEFT JOIN pw ON pw.PWLART=g.PWLART
LEFT JOIN LEB l      ON LTRIM(RTRIM(l.LEBID))=LTRIM(RTRIM(b.LEBID))
LEFT JOIN PERSONAL p ON LTRIM(RTRIM(p.PID))=LTRIM(RTRIM(l.PID))
WHERE b.GOART='g' AND b.DATUM>='20260101'
GROUP BY p.KUERZEL;
```

## 5. Fünf Fallstricke

1. **GO-Gültigkeit:** `BISDATUM` ist **NULL**, nicht `''` → `(BISDATUM IS NULL OR BISDATUM='' OR BISDATUM>=Datum)`.
2. **`GO.GOART` ≠ `BEH.GOART`** (GO: `g`=zahnärztl., `h`=EBM/GOÄ). Nicht danach filtern — nur über `KYLSTNR` joinen.
3. **`PATNR` 10-stellig rechtsbündig mit Leerzeichen** → immer `LTRIM(RTRIM())`. Gleiches für `LEBID`/`PID`.
4. **Scope:** `GOART='g'` = **nur konservierender BEMA**. `FAKT.ZHON` (GKV) enthält zusätzlich ZE/PAR/KFO/KBR
   → berechnet (310k) ≠ fakturiert (584k) 2026. **Kein Bug, anderer Topf.**
5. **Timing:** `FAKTDATUM` (Rechnung) ≠ `BEH.DATUM` (Behandlung), Quartalsversatz. Für Abgleiche Perioden
   angleichen; Datenrauschen abfangen (z. B. FAKT-Zeile mit Datum `50230509`).

## 6. Verbindung (read-only)

Login `praxishub_ro` (`db_datareader`), Server `srv-fs\z1`, DB `Z1`,
`Encrypt=True;TrustServerCertificate=True`. Passwort DPAPI-verschlüsselt in
`%APPDATA%\praxishub\connector\config\config.json` (`z1_db_password`, Präfix `dpapi:` +
Base64, `CryptUnprotectData` als der Windows-User).
