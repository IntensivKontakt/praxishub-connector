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
| Certificate Profile | ⏳ **offen** — anlegen, sobald Identity Validation freigegeben ist |

> Die GUIDs sind keine Geheimnisse (OIDC = kein Client-Secret). Der in der
> Signatur sichtbare Herausgeber kommt aus der **Identity Validation** (= geprüfter
> Firmenname, z. B. „IntensivKontakt GmbH"), nicht aus dem Account-Namen.

## GitHub-Konfiguration (bereits gesetzt)

- **Secrets:** `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`
- **Variables:** `TRUSTED_SIGNING_ENDPOINT`, `TRUSTED_SIGNING_ACCOUNT`, `TRUSTED_SIGNING_PROFILE`
- **Environment:** `release` (an den OIDC-Credential gebunden)

## Letzter offener Schritt: Certificate Profile (nach Freigabe)

Sobald die Identity Validation auf *Completed/Approved* steht:

```bash
az rest --method put \
  --url "https://management.azure.com/subscriptions/bac98e41-8609-4ccf-b33e-29f8e36d581f/resourceGroups/IntensivKontakt/providers/Microsoft.CodeSigning/codeSigningAccounts/Praxishub/certificateProfiles/praxishub-connector?api-version=2024-09-30-preview" \
  --body '{"properties":{"profileType":"PublicTrust","identityValidationId":"<VALIDATION-ID>"}}'
```

(`<VALIDATION-ID>` = ID der freigegebenen Identitätsvalidierung; danach ggf.
`TRUSTED_SIGNING_PROFILE` auf den Profilnamen setzen, falls abweichend.)

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
