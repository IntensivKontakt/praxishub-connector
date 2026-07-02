# PraxisArchiv-Patienten-Lookup (Weg A) — Name/Vorname/Geburtsdatum -> PatientenID.
#
# Wird vom Connector via 32-bit `powershell.exe -EncodedCommand` aufgerufen (die
# PraxisArchiv-COM-DLL `DBClient.dll` ist ein 32-bit-In-Process-Server; der 64-bit-
# Connector kann sie nicht direkt laden, darum dieser Sidecar-Prozess).
#
# READ-ONLY: ausschließlich SELECT/COUNT über ITables.PerformCountSQL. Es werden
# KEINE schreibenden COM-Methoden deklariert.
#
# Eingabe (Umgebungsvariablen, nicht Kommandozeile → kein Argument-Leak):
#   PA_LAST, PA_FIRST, PA_DOB (Format TT.MM.JJJJ), PA_ZIP (optional, Tiebreaker)
# Ausgabe: genau EINE JSON-Zeile auf stdout:
#   {"status":"found","patient_id":"16006"} | {"status":"none"} |
#   {"status":"ambiguous","count":N} | {"status":"error","message":"…"} |
#   {"status":"unavailable","message":"…"}

$ErrorActionPreference = 'Stop'

$src = @"
using System;
using System.Runtime.InteropServices;

[ComImport, Guid("a80de17f-5d70-4ab9-b1d2-f70ebac27543"),
 InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IDBHandler {
    [PreserveSig] int Connect();                                        // 0
    [PreserveSig] int Disconnect();                                     // 1
    void s2();                                                          // 2 ConnectEx
    [PreserveSig] int GetServer(ref Guid riid,                          // 3
        [MarshalAs(UnmanagedType.IUnknown)] out object ppServer);
}

// ITables IID {22a5a712-...}. Nur PerformCountSQL (Slot 50) wird real genutzt;
// die Slots 0-49 sind leere Platzhalter in exakter Vtable-Reihenfolge (nie
// aufgerufen). Bewusst KEINE der schreibenden Methoden (Ins*/Upd*/Del*).
[ComImport, Guid("22a5a712-bca0-4be3-aa80-500ef97cdccf"),
 InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface ITables {
    void t0();  void t1();  void t2();  void t3();  void t4();
    void t5();  void t6();  void t7();  void t8();  void t9();
    void t10(); void t11(); void t12(); void t13(); void t14();
    void t15(); void t16(); void t17(); void t18(); void t19();
    void t20(); void t21(); void t22(); void t23(); void t24();
    void t25(); void t26(); void t27(); void t28(); void t29();
    void t30(); void t31(); void t32(); void t33(); void t34();
    void t35(); void t36(); void t37(); void t38(); void t39();
    void t40(); void t41(); void t42(); void t43(); void t44();
    void t45(); void t46(); void t47(); void t48(); void t49();
    [PreserveSig] int PerformCountSQL(                                  // 50
        [MarshalAs(UnmanagedType.BStr)] string cmd, out uint scalar);
}

public static class PaLookup {
    static object o; static IDBHandler db; static object srv; static ITables tabs;
    // 0 = ok, sonst HRESULT des Connect.
    public static int Init() {
        Type t = Type.GetTypeFromCLSID(new Guid("F990A614-7D6F-460A-B143-6CCA469E6613"));
        o = Activator.CreateInstance(t);
        db = (IDBHandler)o;
        int hr = db.Connect();                 // ambient Windows-Login, keine Credentials
        if (hr != 0) return hr;
        Guid iidUnk = new Guid("00000000-0000-0000-C000-000000000046");
        db.GetServer(ref iidUnk, out srv);     // IDBServer
        tabs = (ITables)srv;                   // QueryInterface auf ITables
        return 0;
    }
    // Liefert den ersten Skalar der Query, oder -1 bei SQL-/COM-Fehler.
    public static long Count(string sql) {
        uint v = 0;
        int hr = tabs.PerformCountSQL(sql, out v);
        if (hr != 0) return -1;
        return (long)v;
    }
    public static void Done() { try { if (db != null) db.Disconnect(); } catch {} }
}
"@

function Emit($obj) { [Console]::Out.WriteLine( ($obj | ConvertTo-Json -Compress) ) }

# SQL-String-Literal absichern: einfache Anführungszeichen verdoppeln.
function Q([string]$s) { if ($null -eq $s) { return '' } return ($s.Trim() -replace "'", "''") }

try {
    Add-Type -TypeDefinition $src | Out-Null
} catch {
    Emit @{ status = 'unavailable'; message = "COM-Interop nicht ladbar: $($_.Exception.Message)" }
    return
}

try {
    $hr = [PaLookup]::Init()
    if ($hr -ne 0) {
        Emit @{ status = 'unavailable'; message = ("PraxisArchiv-Connect fehlgeschlagen (hr=0x{0:X8})" -f $hr) }
        return
    }

    $last  = Q $env:PA_LAST
    $first = Q $env:PA_FIRST
    $dob   = Q $env:PA_DOB
    $zip   = Q $env:PA_ZIP
    if ($last -eq '' -or $dob -eq '') {
        Emit @{ status = 'error'; message = 'Nachname und Geburtsdatum erforderlich' }
        return
    }

    # Primärschlüssel: Nachname + Geburtsdatum (+ Vorname, wenn vorhanden).
    $base = "Name='$last' AND Geburtsdatum='$dob'"
    if ($first -ne '') { $base += " AND Vorname='$first'" }

    $c = [PaLookup]::Count("SELECT COUNT(*) FROM AG1_MasterData WHERE $base")
    if ($c -lt 0) { Emit @{ status = 'error'; message = 'COUNT-Query fehlgeschlagen' }; return }
    if ($c -eq 0) { Emit @{ status = 'none' }; return }

    # Mehrere Treffer → per Postleitzahl eingrenzen (Tiebreaker). Schlägt die
    # PLZ-Query fehl oder grenzt nicht auf genau 1 ein, bleibt es mehrdeutig.
    if ($c -gt 1 -and $zip -ne '') {
        $w = "$base AND Postleitzahl='$zip'"
        $c2 = [PaLookup]::Count("SELECT COUNT(*) FROM AG1_MasterData WHERE $w")
        if ($c2 -eq 1) { $base = $w; $c = 1 }
    }

    if ($c -eq 1) {
        # PatientenID ist numerisch → kommt als Skalar zurück.
        $id = [PaLookup]::Count("SELECT PatientenID FROM AG1_MasterData WHERE $base")
        if ($id -le 0) { Emit @{ status = 'error'; message = 'PatientenID-Abruf fehlgeschlagen' }; return }
        Emit @{ status = 'found'; patient_id = ([string]$id) }
        return
    }

    Emit @{ status = 'ambiguous'; count = $c }
}
catch {
    Emit @{ status = 'error'; message = $_.Exception.Message }
}
finally {
    try { [PaLookup]::Done() } catch {}
}
