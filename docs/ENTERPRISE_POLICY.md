# Managed Policy (central deployment)

IT pins every install to one AI summary model, one transcription model and language,
IT-supplied model files, and an org template library, with a single JSON file the app
reads at launch. Everything it controls is hidden from users.

| OS | Path |
|----|------|
| Windows | `%ProgramData%\Noetis\policy.json` |
| macOS | `/Library/Application Support/Noetis/policy.json` |
| Linux | `/etc/noetis/policy.json` |

## Quick start (Windows, Azure AI Foundry + DeepSeek)

1. **Azure:** deploy `DeepSeek-V4-Flash-0731` in your Foundry resource. Note the endpoint
   (`https://<resource>.services.ai.azure.com/openai/v1`), deployment name and key.
2. **Stage models once** on an admin machine (downloads and SHA-256-verifies them):
   ```powershell
   .\docs\fetch-models.ps1 -Destination \\fileserver\noetis-models
   ```
3. **Each PC** (Intune Win32 app / platform script as SYSTEM, after installing Noetis):
   ```powershell
   .\docs\deploy-policy.ps1 -Endpoint "https://<resource>.services.ai.azure.com/openai/v1" `
       -Model "DeepSeek-V4-Flash-0731" -ApiKey "<key>" `
       -ModelsSource "\\fileserver\noetis-models" -Language en
   ```
   You can also ship the staged `models` folder inside your own install package and pass
   its local path as `-ModelsSource`.

Users then get no onboarding, no model downloads, no model or language settings, no
analytics switch — summaries go to DeepSeek and transcription uses the supplied model.

## policy.json reference

```json
{
  "summary": {
    "endpoint": "https://<resource>.services.ai.azure.com/openai/v1",
    "model": "DeepSeek-V4-Flash-0731",
    "apiKey": "dpapi:v1:<hex written by deploy-policy.ps1>",
    "maxTokens": 8192
  },
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
| `summary` | All summaries use this OpenAI-compatible endpoint. Model settings and the meeting "AI Model" button are hidden; the key never reaches the UI. Azure hosts get the `api-key` header. `apiKey` may be `dpapi:v1:…` (machine-scope DPAPI, decrypted on that PC only), plaintext, or omitted when a gateway adds auth. |
| `transcription` | `provider` = `parakeet` or `localWhisper`; `model` = e.g. `parakeet-tdt-0.6b-v3-int8` or `large-v3-turbo`; optional `language` (e.g. `en`; omit = auto-detect). Applies to live recording, re-transcribe and import. Onboarding is skipped; the Transcription settings tab and language/model pickers are hidden. |
| `modelsDir` | Machine-wide folder of IT-supplied models, checked before the per-user one. Layout: `ggml-<name>.bin` (Whisper), `parakeet/<model>/…`, `diarization/voxceleb_resnet34.onnx` + `diarization/segmentation-3.0.onnx` (speaker identification). |
| `allowModelDownloads` | `false` blocks every model download (transcription, built-in AI, speaker model). Default `true`. |
| `disableAnalytics` | `true` forces usage analytics off and hides the consent switch. |
| `templatesDir` | Org template library (template `.json` files), shown to everyone and overriding user/built-in templates with the same id. |

A file that exists but is invalid (bad JSON, unknown key, bad value) **fails closed**:
summaries refuse to run, downloads and analytics are off, and Settings shows an error.
Policy changes apply on the next app launch.

## Key handling — what it does and doesn't protect

The DPAPI-encrypted key never appears in the file in clear text or anywhere in the app,
and a copied `policy.json` won't decrypt on another PC. It does **not** stop someone who
can run their own code on the PC, because the app must decrypt the key to use it. For
that, use keyless Entra ID auth or an Azure API Management gateway that adds the key
server-side (omit `apiKey`). The key is also visible to Intune admins in the script
parameters. Rotate it if a device is lost.

The policy file is a fleet-consistency control, not a boundary against a local
administrator.
