# PraxisArchiv-COM: Referenz für den Patienten-Lookup (Weg A)

Diese Datei hält die reverse-engineerte COM-Schnittstelle von CGM PraxisArchiv
(ConVis, Version 5.2.837.24164) fest, über die der Connector die Z1-`PatientenID`
eines Patienten aus **Name + Geburtsdatum** auflöst — die Voraussetzung, um eine
wartende Anamnese unbeaufsichtigt in die richtige Akte zu legen, wenn das Backend
(noch) keine PatientenID kennt (Doctolib-Neupatienten).

> **Nur auf einem Rechner mit installiertem PraxisArchiv ermittelbar.** Die
> Typbibliothek in `DBClient.dll` lädt zur Laufzeit nicht (`LoadTypeLibEx` →
> `TYPE_E_CANTLOADLIBRARY`, unaufgelöste abhängige Typelib), und die relevanten
> Interfaces sind **Custom-Interfaces ohne `IDispatch`** — daher aus
> Skriptsprachen (PowerShell) nicht ansprechbar. Die Signaturen unten wurden per
> `IProvideClassInfo → ITypeInfo` am instanziierten Objekt ausgelesen.

## Architektur-Randbedingungen

- **32-bit, in-process:** `DBClient.dll` (`InprocServer32`). Der Connector ist
  64-bit → der Lookup muss in einem **eigenen 32-bit-Sidecar-Prozess** laufen, den
  der Connector als Subprozess aufruft (stdin/stdout-JSON).
- **Kein Direkt-DB-Zugriff:** Die Datenbank liegt hinter CGMs proprietärem
  `DBSrv`-COM-Server auf `\\srv-fs` (Proxy/Stub `DBSrvPS.dll`/`DBClientPS.dll`),
  keine Standard-SQL-Engine lokal. Der COM-Weg über `IDBHandler` ist der einzige.
- **CI kann das Sidecar nicht selbst binden:** GitHub-Runner haben kein
  PraxisArchiv → kein `#import`/`tlbimp` möglich. Die Interfaces müssen daher als
  **vendored Bindings** (aus den Angaben hier) ins Repo, damit die CI ohne PA
  kompiliert.
- **Empfohlene Sidecar-Sprache: C#/.NET.** `dotnet.exe` ist auf dem Praxis-PC
  vorhanden (Rust/C++ nicht) → ein C#-Sidecar lässt sich dort **bauen und live
  testen**. Das Interface wird per `[ComImport, Guid(...), InterfaceType(IUnknown)]`
  in Vtable-Reihenfolge deklariert (Reihenfolge = Funktionsliste unten).

## Der Lookup-Weg (minimal)

`IDBHandler` bietet **`DBQueryToFile`** — ein SQL-SELECT, dessen Ergebnis in eine
Datei geschrieben wird. **`DBQueryToFile` erwies sich als Sackgasse** (fester
Diagnose-Dumper für System-CSV, kein Ad-hoc-SELECT — schreibt für eigene Queries
nichts). Der funktionierende Weg (2026-07-02 **live an Patient 16006 verifiziert**,
read-only) geht über `ITables`:

1. `CoCreateInstance(CGPACS_DBClient.DBHandler)` → `IDBHandler`
2. **`Connect()`** — genügt, **ohne Credentials** (ambient Windows-/Z1-Kontext;
   `hr=0`). `ConnectAsAdm("lwt","0707")` geht auch, ist aber nicht nötig.
3. `GetServer(IID_IUnknown, &srv)` → `srv` = `IDBServer`-Objekt
4. `srv` per QueryInterface auf **`ITables`** casten
5. **`ITables.PerformCountSQL(sql, &scalar)`** — führt ein SQL aus und liefert den
   **ersten Skalar** der ersten Zeile zurück. Damit zwei Nutzungen:
   - `SELECT COUNT(*) FROM AG1_MasterData WHERE …` → Trefferzahl (Eindeutigkeit)
   - `SELECT PatientenID FROM AG1_MasterData WHERE …` → **die PatientenID direkt**
     (PatientenID ist numerisch, z. B. `16006` → passt in den `ulong`-Skalar)
6. `Disconnect()`

**Lookup-Strategie (COUNT-then-fetch, nur `PerformCountSQL` nötig — kein
Enumerator-RE):**
1. `SELECT COUNT(*) … WHERE Name=? AND Vorname=? AND Geburtsdatum=?`
   → **1**: `SELECT PatientenID … WHERE (dieselbe Bedingung)` = das Ergebnis.
   → **0**: Patient (noch) nicht angelegt → zurückstellen, später erneut (Retry).
   → **>1**: Tiebreaker — Bedingung um `PLZ=?` (dann `EMail`) erweitern und erneut
     zählen; genau **1** → PatientenID holen, sonst **nicht ablegen** (mehrdeutig).
   Das entspricht [`core::matching::resolve_unique`](../core/src/matching.rs), hier
   aber durch progressive SQL-Einschränkung realisiert.

**Verifizierte Fakten (Patient Groth/16006):**
- `SELECT PatientenID … WHERE Name='Groth' AND Vorname='Nikolas'` → **16006**.
- `WHERE Name='Groth'` allein → **4** Treffer → Vorname+Geburtsdatum sind Pflicht.
- **Geburtsdatum-Spalte erwartet `TT.MM.JJJJ`:** `'23.02.2001'` → Treffer;
  `'2001-02-23'` → `hr=0x80040E07` (Konvertierungsfehler). **Der Connector muss die
  DOB fürs SQL als `TT.MM.JJJJ` formatieren** (Backend liefert `JJJJMMTT`).
- `ArchiveID`-Filter nicht nötig (kann aber ergänzt werden: `ArchiveID=1`).

> **SQL-Injection:** `Name`/`Vorname` sind patienten-getippt. `PerformCountSQL`
> nimmt rohes SQL → im Sidecar **einfache Anführungszeichen verdoppeln** (`'`→`''`)
> und die Werte in Quotes setzen. Keine anderen Zeichen durchreichen.

> **Streng read-only halten:** Ausschließlich `SELECT`/`COUNT`. `ITables`/`IDBServer`/
> `IDBHandler` können auch schreiben/löschen (Ins*/Upd*/Del*/ExecuteSQLCommand,
> Archive*, SaveMDRecovery* …) — diese Methoden im Sidecar nicht deklarieren/aufrufen.

**Gegenprobe:** `ISimplyArchive.CheckMD('16006', …)`/`ITables.GetMDID` lösen eine
bekannte PatientenID → interne MD-ID auf (das ist `SimpleAR::CheckMD('16006')→5631`
aus dem Trace). Für die Name→ID-Suche nicht nötig.

## `IDBHandler` — IID `{a80de17f-5d70-4ab9-b1d2-f70ebac27543}`

ProgID `CGPACS_DBClient.DBHandler`, CLSID `{F990A614-7D6F-460A-B143-6CCA469E6613}`,
`InprocServer32 = C:\CGM\PRAXISARCHIV\Client\DBClient.dll`. Custom-Interface
(IUnknown-abgeleitet), 76 Methoden. Für den Lookup relevant:

| Methode | Signatur (HRESULT-Rückgabe) | Zweck |
|---|---|---|
| `Connect` | `()` | Verbindung mit Standard-Login |
| `ConnectEx` | `(BSTR sComputer, BSTR sUser, BSTR sPassword)` | Verbindung zu benanntem Server |
| `ConnectAsAdm` | `(BSTR sUser, BSTR sPassword)` | Admin-Login (wie MmoInfIm) |
| `Disconnect` | `()` | Trennen |
| `GetServer` | `(GUID* riid, IUnknown** pServer)` (Slot 3) | liefert `IDBServer` (mit `IID_IUnknown` als riid) → daraus per QI `ITables` |

**Slots (0-basiert, nach IUnknown):** `Connect`=0, `Disconnect`=1, `ConnectEx`=2,
`GetServer`=3, `ConnectAsAdm`=48. Für den Lookup werden nur `Connect`/`GetServer`/
`Disconnect` real deklariert/aufgerufen; alle anderen Slots als leere Stubs
(`void sN();`) in exakter Vtable-Reihenfolge auffüllen. Vollständige Liste (76+47+53+39
Methoden aller vier Interfaces) in [`praxisarchiv-com-vtable.txt`](praxisarchiv-com-vtable.txt).
`vt29`=`VARIANT`/typabhängiger Zeiger, `vt30`=`BSTR`, `vt17`=`byte*`; ungenutzte
Slots als `void` deklarieren, da nie aufgerufen.

## `IDBServer` — IID `{b73d920a-ae53-4848-9be1-7adbcf4fa095}`

Über `IDBHandler.GetServer(IID_IUnknown, &srv)`. 39 Methoden. `ExecuteSQLCommand(ulong
ulID, BSTR sSQL)` führt Nicht-Query-SQL aus (schreibend — **nicht** verwenden). Für
den read-only-Lookup nicht direkt nötig; wir casten `srv` weiter auf `ITables`.

## `ITables` — IID `{22a5a712-bca0-4be3-aa80-500ef97cdccf}`

Per QueryInterface auf dem `IDBServer`-Objekt (`srv as ITables`). 53 Methoden. Für
den Lookup relevant:

| Methode | Slot | Signatur | Zweck |
|---|---|---|---|
| `PerformCountSQL` | 50 | `(BSTR countCommand, ulong* scalarValue)` | **SQL → erster Skalar** (COUNT bzw. PatientenID) |
| `GetTableEnumeratorSQL` | 0 | `(ulong dwID, BSTR sSQL, TblDesc* pTblDesc, IUnknown** ppEnum)` | voller Rowset-Enumerator (nur nötig, wenn mehrere Spalten/Zeilen gebraucht werden — Enumerator-Interface dann noch zu reversen) |
| `GetMDID` | 41 | `(ulong dwID, ushort ushArchiveID, BSTR sExternalID, long* lID)` | PatientenID → interne MD-ID |
| `GetExternalMDID` | 32 | `(ulong dwID, ushort ushArchiveID, long lMasterData, BSTR* sID)` | interne MD-ID → PatientenID |

Für den Namens-Lookup reicht `PerformCountSQL` (COUNT-then-fetch, s. o.). Der
Enumerator-Weg (`GetTableEnumeratorSQL`) wäre nötig, um mehrere Kandidaten inkl.
PLZ/E-Mail in einem Rutsch zu lesen — für die Tiebreaker genügt aber progressives
Einschränken per zusätzlicher `PerformCountSQL`.

## `ISimplyArchive` — IID `{64bc95da-0bd9-45fc-b7da-9b7e50ce0b69}`

ProgID `CGPACS_DataSrc.SimplyArchive`. 47 Methoden. Nur für die Gegenprobe relevant:

| Methode | Signatur | Zweck |
|---|---|---|
| `CheckMD` | `(BSTR sMDID, long* lMDID, void** pMD)` | PatientenID (extern) → interne MD-ID |

## Stammdaten-Felder (`AG1_MasterData`)

Aus `CGPACS_XCH.AG1MasterDataRecord` (IDispatch, 85 Properties) — die für Match &
Tiebreaker nutzbaren Spalten. Deutsche Spaltennamen aus dem MmoInfIm-SQL/Trace:
`PatientenID`, `Name`, `Vorname`, `Geburtsdatum`, `PLZ`/`ZipCode`, `Ort`/`City`,
`Strasse`/`Street`, E-Mail (`HomeEmail`/`BusinessEmail`), `CellPhone`/`HomePhone`,
`Geschlecht`, `Insurance`/Versicherung. Backend liefert für den Match inzwischen
Name, Vorname, Geburtsdatum (Z1-Format `JJJJMMTT`), PLZ und E-Mail.

Primär-Match: **Nachname + Vorname + Geburtsdatum** (normalisiert). Bei
Namensvettern entscheiden **PLZ**, dann **E-Mail**; bleibt es mehrdeutig →
**nicht ablegen** (siehe `core::matching::resolve_unique`).

## Status & verbleibende Schritte

**Verifiziert (2026-07-02, live, read-only):** Login-Modus (`Connect()` ohne
Credentials), COM-Kette, `PerformCountSQL`, Tabellen-/Spaltennamen, Datumsformat
`TT.MM.JJJJ`, Name→PatientenID an Patient 16006. Kein Schreibzugriff, DB unverändert.

**Noch zu bauen:**
- **Sidecar `pa-lookup` (C#/.NET, x86):** die vier `[ComImport]`-Interfaces oben,
  COUNT-then-fetch-Logik, JSON-I/O (stdin: Name/Vorname/GebDat/PLZ/Email; stdout:
  PatientenID | `none` | `ambiguous`), `'`→`''`-Escaping. In der CI baubar (nur
  vendored Bindings, kein PraxisArchiv nötig); live testbar auf dem Praxis-PC.
- **Connector-Kaskade:** ruft das Sidecar für Dokumente ohne `patient_id`, füttert
  das Ergebnis in die bestehende PATID-Push-Strecke.
- **Produktions-Auth:** `Connect()` genügt (läuft im Kontext des angemeldeten,
  PA-berechtigten Praxis-Nutzers) → keine PA-Credentials in der Config nötig.
