# Network Allowlist

Derived from `security/reports/02-network-audit.md` (static analysis, not yet runtime-verified — see `RESIDUAL_RISKS.md` for the Phase 10 gap). This is the set of destinations Noetis's shipped Tauri app can legitimately reach. Anything outside this list observed during a future runtime test (Phase 10) should be treated as unexpected and investigated.

## Local loopback (always essential)

| Destination | Purpose |
|---|---|
| `http://localhost:5167` | Bundled local backend API (meeting/transcript CRUD) |
| `http://127.0.0.1:8178/stream` | Local transcript streaming |
| `http://localhost:11434` (default, user-configurable) | Local Ollama API — see note below |

## Optional external — user-initiated, requires explicit provider selection + API key

| Destination | Trigger | Data sent |
|---|---|---|
| `https://api.openai.com/v1/chat/completions`, `/v1/models` | User selects OpenAI as summary provider | Transcript text (chat), none (models list) |
| `https://api.anthropic.com/v1/messages`, `/v1/models` | User selects Claude as summary provider | Transcript text (messages), none (models list) |
| `https://api.groq.com/openai/v1/chat/completions`, `/v1/models` | User selects Groq as summary provider | Transcript text (chat), none (models list) |
| `https://openrouter.ai/api/v1/chat/completions`, `/v1/models` | User selects OpenRouter as summary provider | Transcript text (chat), none (models list) |
| User-supplied custom OpenAI-compatible endpoint | User configures Custom OpenAI provider | Transcript text |
| User-supplied Ollama endpoint (if changed from default) | User sets a non-default Ollama endpoint | Transcript text — **not currently restricted to loopback; see RESIDUAL_RISKS.md** |

## Model / binary downloads — explicit user action, one-time per model

| Destination | Purpose |
|---|---|
| `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-*.bin` | Whisper model download |
| `https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/...` | Parakeet v2 model download |
| `https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/8f23f0c0.../*` | Parakeet v3 model download — pinned commit, all 4 artifacts SHA-256 pinned |
| `https://huggingface.co/unsloth/...`, `https://huggingface.co/bartowski/...` | On-device (Qwen/Gemma) GGUF model downloads |
| `https://huggingface.co/Wespeaker/wespeaker-voxceleb-resnet34/resolve/ff1ac5bc.../voxceleb_resnet34.onnx` | Speaker-identification model, fetched on first "Speakers" click — pinned commit, size + SHA-256 pinned, blocked in Strict Offline Mode |
| Summary endpoint from managed `policy.json` (e.g. `https://<resource>.services.ai.azure.com/openai/v1`) | Org-enforced summary provider when IT deploys a policy — see docs/ENTERPRISE_POLICY.md |
| `https://github.com/binesh-balan/ffmpeg-binaries/releases/download/0.0.1/...` | Build-time FFmpeg binary fetch |
| `https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/...` | Build-time ONNX Runtime fetch |

## Update service

| Destination | Purpose |
|---|---|
| `https://github.com/binesh-balan/noetis/releases/latest/download/latest.json` | Tauri auto-updater manifest check (fires on every app startup) — this fork's releases; signing key still pending, see RESIDUAL_RISKS.md |

## Telemetry — opt-in, default OFF

| Destination | Purpose |
|---|---|
| `https://us.i.posthog.com` | Analytics events, only if the user explicitly enables the "Enable Analytics" toggle. PII fields (meeting title, file paths, device name) are stripped before send — see `security/reports/02-network-audit.md` §6. |

## System-browser links (not app HTTP requests)

the project's `PRIVACY_POLICY.md` on GitHub, `https://ollama.com/download` — opened via the OS default browser on explicit user click, not fetched by the app itself.

## Known gaps (do not yet trust this list as complete or enforced)

- No runtime traffic capture has been performed (Phase 10 deferred by owner) — this list is derived from source code, not observed behavior.
- No centralized outbound-request gate exists in the app (`security/reports/03-offline-architecture.md` §5) — this list can't currently be enforced by the app itself, only by external tooling (OS firewall) or a future refactor.
- The Ollama and custom-OpenAI endpoints are user-configurable to any URL with no loopback/allowlist restriction (`security/reports/03-offline-architecture.md` §3) — a user (or a socially-engineered paste) can point either at an arbitrary host.
