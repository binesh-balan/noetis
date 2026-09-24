# Managed Policy (central deployment)

IT pins every install to one AI summary model, one transcription model and language,
IT-supplied model files, and an org template library, with a single JSON file the app
reads at launch. Everything it controls is hidden from users.

| OS | Path |
|----|------|
| Windows | `%ProgramData%\Noetis\policy.json` |
| macOS | `/Library/Application Support/Noetis/policy.json` |
| Linux | `/etc/noetis/policy.json` |

## Quick start (Windows, Azure AI Foundry + DeepSeek, no keys on devices)

Users sign in with their work account (Entra ID). The app gets a short-lived token for your
Foundry resource; no API key or client secret is ever on a user's PC, access follows group
membership, and Azure logs show who made each call.

### One-time Azure setup (tenant admin)

1. **Model:** deploy `DeepSeek-V4-Flash-0731` in your Foundry (Azure AI services) resource.
   Note the endpoint (`https://<resource>.services.ai.azure.com/openai/v1`) and deployment name.
2. **App registration** (Entra admin center → App registrations → New registration):
   - Name `Noetis`, *Accounts in this organizational directory only*.
   - Authentication → Add a platform → **Mobile and desktop applications** → custom redirect
     URI `http://localhost` (Entra allows any port on localhost for desktop apps).
   - No client secret or certificate (it's a public client using PKCE).
   - API permissions → Add → *APIs my organization uses* → **Azure Cognitive Services** →
     Delegated → `user_impersonation` → **Grant admin consent** (so users aren't asked).
   - Note the **Application (client) ID** and **Directory (tenant) ID**.
3. **Who may use it:** create a security group (e.g. *Noetis users*). On the Foundry resource →
   Access control (IAM) → Add role assignment → **Cognitive Services User** → the group.
   Role changes can take a few minutes to apply. (Verify the role against Microsoft's current
   Foundry RBAC documentation if calls return 401/403.)
4. **Optional, recommended:** disable key-based access on the resource (local auth off), so
   Foundry keys cannot be used by anyone.

### Deploy

1. **Stage models once** on an admin machine (downloads and SHA-256-verifies them):
   ```powershell
   .\docs\fetch-models.ps1 -Destination \fileserver\noetis-models
   ```
2. **Each PC** (Intune Win32 app / platform script as SYSTEM, after installing Noetis), one line:
   ```powershell
   .\docs\deploy-policy.ps1 -Endpoint "https://<resource>.services.ai.azure.com/openai/v1" -Model "DeepSeek-V4-Flash-0731" -TenantId "<directory id>" -ClientId "<application id>" -ModelsSource "\fileserver\noetis-models" -Language en
   ```
   You can also ship the staged `models` folder inside your own install package and pass
   its local path as `-ModelsSource`.

Users then get no onboarding, no model downloads, no model or language settings, no
analytics switch. The first summary opens the Microsoft sign-in page (on Entra-joined PCs
usually a single click, no password); after that tokens renew silently. Settings → General
shows the signed-in account with Sign in / Sign out.

## policy.json reference

```json
{
  "summary": {
    "endpoint": "https://<resource>.services.ai.azure.com/openai/v1",
    "model": "DeepSeek-V4-Flash-0731",
    "maxTokens": 8192
  },
  "entra": { "tenantId": "<directory id>", "clientId": "<application id>" },
  "transcription": { "provider": "parakeet", "model": "parakeet-tdt-0.6b-v3-int8", "language": "en" },
  "modelsDir": "C:\\ProgramData\\Noetis\\models",
  "allowModelDownloads": false,
  "disableAnalytics": true,
  "templatesDir": "\\\\fileserver\\noetis\\templates"
}
```

Every key is optional.

| Key | Effect |
|---|---|
| `summary` | All summaries use this OpenAI-compatible endpoint. Model settings and the meeting "AI Model" button are hidden. Authentication: `entra` (recommended), or `apiKey` — `dpapi:v1:…` (machine-scope DPAPI, decrypted on that PC only) or plaintext, sent as the `api-key` header to Azure hosts — or neither when a gateway adds auth. |
| `entra` | Keyless summary auth: `tenantId` and `clientId` of the app registration above; optional `scope` (default `https://cognitiveservices.azure.com/.default`) and `authority` (default `https://login.microsoftonline.com`, change for sovereign clouds). Users sign in with their work account; requests carry `Authorization: Bearer <token>`. The access token lives in memory; the refresh token is stored encrypted for that Windows user (DPAPI) in the app data folder. |
| `transcription` | `provider` = `parakeet` or `localWhisper`; `model` = e.g. `parakeet-tdt-0.6b-v3-int8` or `large-v3-turbo`; optional `language` (e.g. `en`; omit = auto-detect). Applies to live recording, re-transcribe and import. Onboarding is skipped; the Transcription settings tab and language/model pickers are hidden. |
| `modelsDir` | Machine-wide folder of IT-supplied models, checked before the per-user one. Layout: `ggml-<name>.bin` (Whisper), `parakeet/<model>/…`, `diarization/voxceleb_resnet34.onnx` + `diarization/segmentation-3.0.onnx` (speaker identification). |
| `allowModelDownloads` | `false` blocks every model download (transcription, built-in AI, speaker model). Default `true`. |
| `disableAnalytics` | `true` forces usage analytics off and hides the consent switch. |
| `templatesDir` | Org template library (template `.json` files), shown to everyone and overriding user/built-in templates with the same id. |

A file that exists but is invalid (bad JSON, unknown key, bad value) **fails closed**:
summaries refuse to run, downloads and analytics are off, and Settings shows an error.
Policy changes apply on the next app launch.

## Credentials — what each option protects

- **Entra ID sign-in (recommended):** nothing secret is deployed. A user's access token is
  short-lived and tied to their account and group membership; removing them from the group
  (or disabling the account) ends access. A stolen laptop exposes only that user's own
  refresh token, which you can revoke in Entra ID.
- **`apiKey` (DPAPI-encrypted):** the key never appears in the file in clear text or in the
  app, and a copied `policy.json` won't decrypt on another PC. It does **not** stop someone
  who can run their own code on the PC, because the app must decrypt the key to use it, and
  one key is shared by everyone. It is also visible to Intune admins in the script parameters.

The policy file is a fleet-consistency control, not a boundary against a local
administrator.
