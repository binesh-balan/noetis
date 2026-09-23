# Managed Policy (central deployment)

IT can pin every install to one summary model and one template library by deploying a
JSON file. The app reads it once at launch.

| OS | Path |
|----|------|
| Windows | `%ProgramData%\Noetis\policy.json` |
| macOS | `/Library/Application Support/Noetis/policy.json` |
| Linux | `/etc/noetis/policy.json` |

```json
{
  "summary": {
    "endpoint": "https://<resource>.services.ai.azure.com/openai/v1",
    "model": "DeepSeek-V4-Flash-0731",
    "apiKey": "<Foundry key, or omit when a gateway injects auth>",
    "maxTokens": 8192,
    "temperature": 0.3
  },
  "templatesDir": "\\\\fileserver\\noetis\\templates"
}
```

Both keys are optional.

- **`summary`** — every summary goes to this OpenAI-compatible endpoint, whatever the user
  picked. The Summary Model settings become a read-only "Managed by your organization"
  card. For `*.azure.com` hosts the key is sent as the `api-key` header.
- **`templatesDir`** — a folder of template `.json` files (same format as
  `frontend/src-tauri/templates/*.json`) shown to every user as the org template library.
  Org templates override user and built-in templates with the same id.

A file that exists but is invalid (bad JSON, unknown key, non-http endpoint) **fails
closed**: summaries refuse to run and Settings shows the error, so a typo never silently
lets users fall back to their own providers.

## Azure AI Foundry + DeepSeek

1. Deploy `DeepSeek-V4-Flash-0731` (or `DeepSeek-V3`) in your Foundry resource. A non-reasoning
   model is the right fit for summaries — reasoning models spend tokens on `<think>` output
   the app strips anyway.
2. Endpoint: `https://<resource>.services.ai.azure.com/openai/v1`. Model = the deployment name.
3. Deploy `policy.json` with Intune (Win32 app or a PowerShell script that writes the file),
   GPO file copy, or your MDM. Restrict the file ACL to Administrators-write / Users-read.

## Key handling

A key in `policy.json` is readable by every local user on the machine. For stricter
setups, omit `apiKey` and point `endpoint` at an Azure API Management gateway that
authenticates users (e.g. by client certificate or network) and injects the Foundry key
server-side. Rotate the Foundry key if a device is lost.

The policy file is a fleet-consistency control, not a boundary against a local
administrator or a user who can redirect `%ProgramData%`.
