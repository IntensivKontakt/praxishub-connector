# Tauri signCommand-Wrapper: signiert eine Datei via Microsoft `sign`-Tool +
# Azure Artifact/Trusted Signing (OIDC/azure-cli, kein Secret).
#
# Tauri ruft das hier mit dem (substituierten) Dateipfad als $File auf. `sign`
# wird bewusst ÜBER pwsh ausgeführt (identisch zum verifizierten Smoke-Kontext) —
# der direkte Spawn durch Tauri scheiterte. Ausgabe + Exit-Code werden für die
# Diagnose nach $RUNNER_TEMP/sign.log geschrieben.
param([Parameter(Mandatory = $true)][string]$File)

$log = if ($env:RUNNER_TEMP) { Join-Path $env:RUNNER_TEMP "sign.log" } else { "sign.log" }
"=== sign-wrap: $File ===" | Tee-Object -FilePath $log -Append

& sign code artifact-signing `
  --artifact-signing-endpoint "https://neu.codesigning.azure.net/" `
  --artifact-signing-account "Praxishub" `
  --artifact-signing-certificate-profile "praxishub-connector" `
  --azure-credential-type azure-cli `
  --timestamp-url "http://timestamp.acs.microsoft.com" `
  -v information `
  "$File" 2>&1 | Tee-Object -FilePath $log -Append
$code = $LASTEXITCODE

"=== sign-wrap exit=$code ===" | Tee-Object -FilePath $log -Append
exit $code
