# Code-Signing — Azure Trusted/Artifact Signing

Der Connector wird über **Azure Trusted/Artifact Signing** signiert (kein lokales
Zertifikat/Token; der Schlüssel bleibt in Azure, signiert wird per OIDC aus CI).
Siehe auch Linear **PRA-15**.

## Eingerichtete Ressourcen (Stand: automatisiert per `az`/`gh`)

| Ding | Wert |
|---|---|
| Azure Subscription | `Praxishub` (`bac98e41-8609-4ccf-b33e-29f8e36d581f`) |
| Tenant | `c5c2c37a-679f-4fcf-8f86-51313fa3578f` |
| Resource Group | `IntensivKontakt` |
| Signing Account | `Praxishub` (Region **North Europe**) |
| **Endpoint** | `https://neu.codesigning.azure.net/` |
| App-Registrierung (CI) | `praxishub-connector-signing-ci` |
| Client-ID | `a5cd9665-c665-424b-95bf-be4908caf24a` |
| Rolle auf dem Account | `Artifact Signing Certificate Profile Signer` |
| OIDC Federated Credentials | `repo:IntensivKontakt/praxishub-connector:ref:refs/heads/main` · `…:environment:release` |
| Certificate Profile | ✅ `praxishub-connector` (PublicTrust, **Active**), angelegt 2026-06-15 |
| Identity Validation | ✅ Completed — ID `30a94f10-c762-4ac0-8e3e-75d59447d291` (Herausgeber „IntensivKontakt GmbH & Co. KG") |

> Die GUIDs sind keine Geheimnisse (OIDC = kein Client-Secret). Der in der
> Signatur sichtbare Herausgeber kommt aus der **Identity Validation** (= geprüfter
> Firmenname, z. B. „IntensivKontakt GmbH"), nicht aus dem Account-Namen.

## GitHub-Konfiguration (bereits gesetzt)

- **Secrets:** `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`
- **Variables:** `TRUSTED_SIGNING_ENDPOINT`, `TRUSTED_SIGNING_ACCOUNT`, `TRUSTED_SIGNING_PROFILE`
- **Environment:** `release` (an den OIDC-Credential gebunden)

## Certificate Profile (✅ angelegt 2026-06-15)

Identity Validation freigegeben → Profil per CLI angelegt (für Referenz/Neuanlage):

```bash
az rest --method put \
  --url "https://management.azure.com/subscriptions/bac98e41-8609-4ccf-b33e-29f8e36d581f/resourceGroups/IntensivKontakt/providers/Microsoft.CodeSigning/codeSigningAccounts/Praxishub/certificateProfiles/praxishub-connector?api-version=2025-10-13" \
  --body '{"properties":{"profileType":"PublicTrust","identityValidationId":"30a94f10-c762-4ac0-8e3e-75d59447d291"}}'
```

Status: `provisioningState=Succeeded`, `status=Active`, Herausgeber
„IntensivKontakt GmbH & Co. KG". `TRUSTED_SIGNING_PROFILE` = `praxishub-connector`
(GitHub-Variable gesetzt). **Signing Smoke Test grün** → Kette OIDC → Azure →
gültige Signatur (`Valid`) bestätigt.

## Verifizieren

`Signing Smoke Test` (Actions-Tab → Run workflow) signiert eine Dummy-.exe und
prüft die ganze Kette OIDC → Azure → Signatur — unabhängig vom Connector-Code.
Grün = das Signing-Setup steht.

## Hinweise

- **Immer mit Zeitstempel** (in den Workflows konfiguriert) — sonst werden Builds
  ungültig, sobald das Zertifikat rotiert.
- **Beide Ebenen signieren:** Installer **und** App-.exe. Sauberster Weg bei Tauri:
  `bundle > windows > signCommand` auf einen Trusted-Signing-Aufruf zeigen lassen,
  sodass auch die inneren Binaries signiert werden (alternativ Post-Build über die
  Action auf den `bundle`-Ordner, wie in `build-sign.yml`).
- Action-Version (`azure/trusted-signing-action`) bei Bedarf auf neueste bumpen.

## Bekannte Grenzen (Stand 2026-06-15)

- **Installer ist signiert** (Azure Trusted Signing, OIDC, Herausgeber
  „IntensivKontakt GmbH & Co. KG") — das ist der SmartScreen-relevante Teil für
  die Verteilung. ✅
- **Innere App-`.exe` noch NICHT signiert.** Der saubere Weg (Tauri
  `bundle.windows.signCommand`, der die App-.exe *während* des Builds signiert)
  scheiterte reproduzierbar an einem **`%1`-Substitutions-Bug der Tauri-CLI**:
  der Platzhalter `%1` wird literal an das Signier-Tool übergeben (Diagnose:
  `Some files do not exist … \%1`), in String- **und** Objekt-Form. 5 Release-Builds
  bestätigt. Folge: nur Post-Build-Signierung des Installers möglich.
  - **Auth-Nebenfund:** `trusted-signing-cli` (gängiges signCommand-Tool) braucht
    `AZURE_CLIENT_SECRET` und ist mit unserem OIDC-Setup inkompatibel. Microsofts
    `sign code artifact-signing --azure-credential-type azure-cli` funktioniert per
    OIDC (im Smoke verifiziert) — aber eben nur, wenn Tauri `%1` ersetzt.
  - **Optionen zum Nachziehen:** (a) Tauri-CLI-Update abwarten/prüfen, ob der
    `%1`-Bug behoben ist; (b) zweiphasiger Build (`tauri build --no-bundle` →
    App-.exe per Action signieren → `tauri bundle` → Installer signieren → `.sig`
    neu); (c) Client-Secret + `trusted-signing-cli` als signCommand (verwirft OIDC).
- **Updater-`.sig`:** wird nach dem Authenticode-Signieren über den signierten
  Installer **neu erzeugt** (`tauri signer sign`), damit die Auto-Update-Prüfung passt.
