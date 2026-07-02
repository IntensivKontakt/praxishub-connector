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
Datei geschrieben wird. Damit ist der Lookup ein Dreisatz, ohne die
Server-/Tabellen-Objekte zu navigieren:

1. `CoCreateInstance(CGPACS_DBClient.DBHandler)` → `IDBHandler`
2. `ConnectAsAdm(user, password)` **oder** `Connect()` (Login-Kontext klären —
   im Trace nutzt CGMs eigener Code „ForceLoginAsAdmin"; Credentials = Z1-Login).
3. `DBQueryToFile(outPath, "SELECT PatientenID, Name, Vorname, Geburtsdatum, PLZ, ... FROM AG1_MasterData WHERE ArchiveID=1 AND ...", 256)`
4. Ergebnisdatei parsen, mit [`core::matching`](../core/src/matching.rs)
   (`normalize_name`/`normalize_birthdate`/`resolve_unique`) gegen den gesuchten
   Patienten abgleichen → genau **eine** `PatientenID` oder „mehrdeutig"/„keiner".
5. `Disconnect()`.

**Gegenprobe/Alternative:** `ISimplyArchive.CheckMD(externalMDID, &lMDID, &pMD)`
löst eine bekannte `PatientenID` (externe MD-ID) → interne `lMDID` auf (das ist
`SimpleAR::CheckMD('16006') → ID 5631` aus dem MmoInfIm-Trace). Nützlich zum
Verifizieren, nicht für die Name→ID-Suche.

> **Streng read-only halten:** Für den Lookup ausschließlich `SELECT` verwenden.
> `IDBHandler`/`ISimplyArchive` können auch schreiben (Archive*, SaveMDRecovery*,
> InsFolder …) — diese Methoden im Sidecar nicht aufrufen.

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
| `DBQueryToFile` | `(BSTR sFileName, BSTR sSQLSelect, ushort ushElInBuffer)` | **SQL-SELECT → Datei** |
| `GetServer` | `(GUID* riid, IUnknown** pServer)` | tieferer DB-Server-Zugriff (falls DBQueryToFile nicht reicht) |
| `GetPatientDataEx` | `(ushort ArchiveID, long masterDataId, IUnknown** ppMasterDataRecord)` | Stammsatz per interner MD-ID |
| `GetDBVersion` | `(short* dbVersion, short* dbRevision)` | Versions-Check |
| `GetDBMS` | `(VARIANT* currentDBMS)` | DBMS-Typ ermitteln |

(Vtable-Reihenfolge für die C#-Deklaration: die vollständige Funktionsliste steht
in [`praxisarchiv-com-vtable.txt`](praxisarchiv-com-vtable.txt) — alle 76 Einträge in exakter Reihenfolge,
inklusive der Schreibmethoden, die deklariert, aber NICHT aufgerufen werden. `vt29`
= `VARIANT`/typ­abhängiger Zeiger, `vt30` = `BSTR`-artig; im Zweifel als `IntPtr`
deklarieren, da für den Lookup nur `DBQueryToFile`/`Connect*`/`Disconnect` real
aufgerufen werden.)

## `ISimplyArchive` — IID `{64bc95da-0bd9-45fc-b7da-9b7e50ce0b69}`

ProgID `CGPACS_DataSrc.SimplyArchive`. 47 Methoden. Für den Lookup relevant:

| Methode | Signatur | Zweck |
|---|---|---|
| `Init` | `(ushort ushArchiveID)` | Archiv wählen (`1`) |
| `UseAdmLogin` | `(bool c)` | Admin-Login aktivieren |
| `CheckMD` | `(BSTR sMDID, long* lMDID, void** pMD)` | PatientenID (extern) → interne MD-ID |
| `GetFolderID` | `(long lMDID, BSTR sFolder, ulong* ulFolderID)` | Ordner-ID (hier „VDDS-Importmodul") |
| `Disconnect` | `()` | Trennen |

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

## Offene Punkte vor dem ersten Live-Lookup

- **Login-Credentials/Modus:** `Connect()` vs. `ConnectAsAdm(user,pass)` —
  welcher Weg headless trägt (der frühere Zerberus-Weg fauliegt den Live-Server;
  `IDBHandler` ist ein anderer, DB-naher Pfad). Z1-Login des Users nutzbar, aber
  **nicht dauerhaft speichern**.
- **Exakte Spaltennamen + Ergebnisformat** von `DBQueryToFile` (Trenner, Encoding)
  → am Testpatienten Groth/16006 verifizieren.
- **ArchiveID** = `1` (aus dem Trace bestätigt).
