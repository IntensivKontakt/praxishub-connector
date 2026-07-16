<#
  Erzeugt ein VOLL-Backup der Live-Z1-DB fuer eine LOKALE Entwicklungs-Kopie.
  - Nur Lesen der Live-DB (BACKUP ... COPY_ONLY): stoert die CGM-Backup-Kette NICHT.
  - Schreibt die .bak-Datei; committet NICHTS. Die Datei/DB-Kopie bleibt lokal (PHI).
  - Restore erfolgt manuell in eine lokale SQL-Server-Developer/Express-Instanz
    (siehe db/README.md) - unsere App-Logins haben keine dbcreator/restore-Rechte.

  Bitte AUSSERHALB der Sprechzeiten ausfuehren (IO-Last auf der Produktiv-Instanz).

  Beispiel:
    powershell -ExecutionPolicy Bypass -File db\make-local-dev-copy.ps1 -BackupDir "\\srv-fs\Backup\z1dev"
    powershell -ExecutionPolicy Bypass -File db\make-local-dev-copy.ps1 -BackupDir "D:\z1dev" -IncludeArchive
#>
param(
  # Fuer den SQL-Server-Dienst SCHREIBBARER Pfad (UNC oder serverlokal auf srv-fs).
  [Parameter(Mandatory=$true)] [string] $BackupDir,
  # Zusaetzlich CGMArchive (PraxisArchiv-Blobs) sichern -> echte PVS-Voll-Kopie.
  [switch] $IncludeArchive
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security

$cfg = Get-Content "$env:APPDATA\praxishub\connector\config\config.json" -Raw | ConvertFrom-Json
function Unprotect($s){ if($s -notlike 'dpapi:*'){return $s}; $b=[Convert]::FromBase64String($s.Substring(6)); $p=[System.Security.Cryptography.ProtectedData]::Unprotect($b,$null,'CurrentUser'); return [System.Text.Encoding]::UTF8.GetString($p) }

$server = $cfg.z1_db_server
$user   = $cfg.z1_db_write_user            # Schreib-Login (hat BACKUP-Recht)
$pass   = Unprotect $cfg.z1_db_write_password
if (-not $user) { throw "Kein Schreib-Login (z1_db_write_user) konfiguriert - fuer BACKUP noetig." }

$stamp = Get-Date -Format 'yyyyMMdd_HHmm'
$dbs = @('Z1')
if ($IncludeArchive) { $dbs += 'CGMArchive' }

$cs = "Server=$server;Database=master;User ID=$user;Password=$pass;Encrypt=True;TrustServerCertificate=True;Connect Timeout=15"
$conn = New-Object System.Data.SqlClient.SqlConnection $cs
$conn.Open()
Write-Host "Verbunden mit $server als $user" -ForegroundColor Green
Write-Host "WICHTIG: BACKUP laeuft serverseitig; '$BackupDir' muss fuer den SQL-Dienst schreibbar sein." -ForegroundColor Yellow

foreach ($db in $dbs) {
  $bak = Join-Path $BackupDir "$($db)_dev_$stamp.bak"
  Write-Host "`n== BACKUP $db -> $bak ==" -ForegroundColor Cyan
  $cmd = $conn.CreateCommand()
  $cmd.CommandTimeout = 0   # kein Timeout (grosse DB)
  # COPY_ONLY: keine Auswirkung auf die regulaere Backup-Kette. COMPRESSION+CHECKSUM.
  $cmd.CommandText = @"
BACKUP DATABASE [$db] TO DISK = N'$bak'
  WITH COPY_ONLY, COMPRESSION, CHECKSUM, INIT, STATS = 5,
       NAME = N'$db Dev-Kopie (COPY_ONLY) $stamp';
"@
  try {
    $cmd.ExecuteNonQuery() | Out-Null
    Write-Host "  OK: $bak" -ForegroundColor Green
  } catch {
    Write-Host "  FEHLER: $($_.Exception.Message.Split([char]10)[0])" -ForegroundColor Red
    Write-Host "  Haeufige Ursache: Pfad fuer den SQL-Dienst nicht schreibbar, oder Rechte." -ForegroundColor DarkGray
  }
}
$conn.Close()

Write-Host "`nNaechste Schritte (lokal, siehe db/README.md):" -ForegroundColor Cyan
Write-Host "  1) .bak in einen gitignorierten Ordner auf den Dev-Rechner kopieren (z.B. db\local-data\)."
Write-Host "  2) In eine LOKALE SQL-Developer/Express-Instanz restoren (RESTORE DATABASE Z1_DEV ...)."
Write-Host "  3) Connector-Config auf die lokale Instanz + Z1_DEV zeigen lassen."
Write-Host "`nHinweis: Die .bak enthaelt echte Patientendaten -> NIEMALS committen/teilen." -ForegroundColor Yellow
