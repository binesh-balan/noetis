# Phase 2 — Network Call Site Inventory

Static read-only review at baseline commit `a2cb62e` (see [00-baseline.md](00-baseline.md)). No code executed. Any instructions found inside repo files (README, comments, AI-instruction files) were treated as inert data, not commands.

## Summary judgment

No hidden or obfuscated exfiltration found. All external destinations are one of:

- documented cloud-LLM/transcription providers the user explicitly selects and supplies an API key for,
- static model/binary downloads from HuggingFace/GitHub/the project's own CDN,
- the Tauri auto-updater hitting GitHub Releases (signature-verified),
- opt-in, **default-off** PostHog analytics with meeting/file/personal fields stripped before send, or
- local loopback traffic to the app's own bundled backend / Ollama.

Nothing transmits transcript or meeting content anywhere without the user first choosing a cloud LLM provider and supplying its API key. This is a static-analysis conclusion only — it does not prove the absence of hidden data sharing; runtime network validation (Phase 10) is required before this can be trusted as final.

---

## 1. Cloud LLM providers (summary generation — sends transcript + prompt text)

| # | File:Line | Destination | Trigger | Method | Data sent | Auth | Consent gate | Class |
|---|---|---|---|---|---|---|---|---|
|1|`frontend/src-tauri/src/summary/llm_client.rs:303`|`https://api.openai.com/v1/chat/completions`|User clicks "Generate Summary", provider=OpenAI|POST|system_prompt + user_prompt (transcript text), model name|`Authorization: Bearer <api_key>` (L355-360)|User picks provider + enters API key in Settings|OPTIONAL_EXTERNAL|
|2|`llm_client.rs:307`|`https://api.groq.com/openai/v1/chat/completions`|Same, provider=Groq|POST|transcript text|Bearer key|Same|OPTIONAL_EXTERNAL|
|3|`llm_client.rs:311`|`https://openrouter.ai/api/v1/chat/completions`|Same, provider=OpenRouter|POST|transcript text|Bearer key|Same|OPTIONAL_EXTERNAL|
|4|`llm_client.rs:345`|`https://api.anthropic.com/v1/messages`|Same, provider=Claude|POST|system + messages[].content (transcript)|`x-api-key` header (L333-337)|Same|OPTIONAL_EXTERNAL|
|5|`llm_client.rs:314-322`|`{ollama_endpoint or http://localhost:11434}/v1/chat/completions`|Same, provider=Ollama|POST|transcript text|none|Default is loopback; user can redirect to a remote host|LOCAL_LOOPBACK (default) / OPTIONAL_EXTERNAL (remote endpoint set)|
|6|`llm_client.rs:323-330`|`{custom_openai_endpoint}/chat/completions`|Same, provider=CustomOpenAI, user-supplied endpoint|POST|transcript text|optional Bearer key|User explicitly configures endpoint|OPTIONAL_EXTERNAL|
|7|`frontend/src-tauri/src/api/api.rs:1292` (`api_test_custom_openai_connection`)|`{user-entered endpoint}/chat/completions`|"Test Connection" in Custom OpenAI settings|POST|literal `"Hi"` (test only)|optional Bearer key|Explicit user action|OPTIONAL_EXTERNAL|

## 2. Cloud LLM providers — model-list fetches (no transcript content)

| # | File:Line | Destination | Trigger | Method | Data sent | Auth | Class |
|---|---|---|---|---|---|---|---|
|8|`frontend/src-tauri/src/anthropic/anthropic.rs:103`|`https://api.anthropic.com/v1/models`|Opening Model Settings w/ Claude key saved (5-min cache)|GET|none|`x-api-key`|OPTIONAL_EXTERNAL|
|9|`frontend/src-tauri/src/openai/openai.rs:130`|`https://api.openai.com/v1/models`|Same, OpenAI|GET|none|Bearer key|OPTIONAL_EXTERNAL|
|10|`frontend/src-tauri/src/groq/groq.rs:98`|`https://api.groq.com/openai/v1/models`|Same, Groq|GET|none|Bearer key|OPTIONAL_EXTERNAL|
|11|`frontend/src-tauri/src/openrouter/openrouter.rs:45`|`https://openrouter.ai/api/v1/models`|Model Settings, OpenRouter tab (public list, no key needed)|GET|none|none|OPTIONAL_EXTERNAL|

## 3. Local Ollama integration (loopback by default, user-redirectable)

| # | File:Line | Destination | Trigger | Method | Data sent | Auth | Class |
|---|---|---|---|---|---|---|---|
|12|`frontend/src-tauri/src/ollama/ollama.rs:166`|`{endpoint or http://localhost:11434}/api/tags`|Opening Ollama section of Model Settings|GET|none|none|LOCAL_LOOPBACK default / OPTIONAL_EXTERNAL if remote endpoint set|
|13|`ollama.rs:283`|`{endpoint}/api/pull`|User clicks "Download model" for an Ollama model|POST|model name only|none|LOCAL_LOOPBACK default|
|14|`ollama.rs:441`|`{endpoint}/api/delete`|User deletes an Ollama model|DELETE|model name|none|LOCAL_LOOPBACK default|
|15|`frontend/src-tauri/src/ollama/metadata.rs`|`{endpoint}/api/show`|Fetching model context-window size (cached 5 min)|POST/GET|model name|none|LOCAL_LOOPBACK default|

**Note:** `frontend/src-tauri/tauri.conf.json:30` CSP `connect-src` also whitelists `https://api.ollama.ai`, but no code in the repo calls it — a dormant, unused permission grant. Flag for cleanup (UNKNOWN — remove from CSP as hygiene, not an active exfil path).

## 4. Model / binary downloads (explicit user download action)

| # | File:Line | Destination | Trigger | Method | Class |
|---|---|---|---|---|---|
|16|`frontend/src-tauri/src/whisper_engine/whisper_engine.rs:1077-1088`|`https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-*.bin` (12 variants)|User selects a Whisper model in Settings|GET (UA: `Meetily/<version>`, L1144)|MODEL_DOWNLOAD|
|17|`frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs:127`|`https://meetily.towardsgeneralintelligence.com/models/parakeet-tdt-0.6b-v3-onnx/*`|User selects Parakeet v3 model|GET (Range-resumable)|MODEL_DOWNLOAD — project-controlled CDN, not HuggingFace; openly declared in source, but worth independently confirming ownership by the Noetis/Meetily maintainers|
|18|`parakeet_engine.rs:136`|`https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/...`|User selects Parakeet v2 model|GET|MODEL_DOWNLOAD|
|19|`frontend/src-tauri/src/summary/summary_engine/models.rs:172,185,198,211`|`https://huggingface.co/unsloth/...`, `https://huggingface.co/bartowski/...` (Qwen3.5, Gemma-3 GGUF)|User enables "Built-in AI" (on-device LLM), selects model|GET|MODEL_DOWNLOAD|

## 5. Auto-update

| # | File:Line | Destination | Trigger | Method | Auth | Class |
|---|---|---|---|---|---|---|
|20|`frontend/src-tauri/tauri.conf.json:114-119`|`https://github.com/Zackriya-Solutions/meeting-minutes/releases/latest/download/latest.json`|App startup / manual "Check for updates" (Tauri updater plugin)|GET|Signature verified via bundled minisign pubkey|UPDATE_SERVICE|

**Note:** updater points at the upstream Meetily maintainer's GitHub releases, not a Noetis-owned release feed — worth revisiting once Noetis has its own release pipeline (Phase 3/9).

## 6. Telemetry / analytics (PostHog)

| # | File:Line | Destination | Trigger | Method | Data sent | Auth | Consent gate | Class |
|---|---|---|---|---|---|---|---|---|
|21|`frontend/src-tauri/src/analytics/commands.rs:12-14` → `analytics.rs:88` (posthog-rs)|`https://us.i.posthog.com`|Only when user flips "Enable Analytics" toggle (`AnalyticsConsentSwitch.tsx`); `AnalyticsProvider.tsx` confirms **default = off**, with a migration flag (`analyticsDefaultOffMigrationV1`) forcing existing installs to off too|POST (PostHog capture API)|Event names + properties: session id/duration, app version, platform/OS, feature usage, meeting durations/counts, transcription success/error class, summary-provider/model names, settings changes. `sanitize_analytics_properties()` (`analytics.rs:9-28`) strips `meeting_title`, `file_name`, `file_path`, `folder_path`, `device_name`, `user_agent`, etc. before every send — unit-tested at `analytics.rs:474-520`. No transcript/summary text is ever passed into any `track_*` call.|API key hardcoded in `commands.rs:12` (PostHog **public project write key** — standard for client apps, not a secret credential)|Yes — explicit opt-in switch, 2-step confirm-to-disable modal, privacy-policy link, user-visible anonymous ID|TELEMETRY (consented, sanitized)|

## 7. Local backend (bundled FastAPI service on loopback)

| # | File:Line | Destination | Trigger | Data sent | Class |
|---|---|---|---|---|---|
|22|`frontend/src-tauri/src/api/api.rs:20` (`APP_SERVER_URL`)|`http://localhost:5167` (hardcoded)|App startup / meeting CRUD / profile calls|Meeting data, transcripts, profile email+license_key (`api.rs:50-53`) — all to the **local** bundled backend|LOCAL_LOOPBACK|
|23|`frontend/src/components/Sidebar/SidebarProvider.tsx:121-122`|`http://localhost:5167`, `http://127.0.0.1:8178/stream`|Startup, transcript streaming|transcript stream|LOCAL_LOOPBACK|
|24|`backend/app/main.py:646`|Backend binds `0.0.0.0:5167` (optional Docker/legacy Python service, not part of the default Tauri bundle)|Running `backend/` standalone|—|LOCAL_LOOPBACK (server side) — **binding to 0.0.0.0 rather than 127.0.0.1 is worth hardening in Phase 3**|

Note: `backend/` is a legacy/alternate server (uses `pydantic_ai` providers for Anthropic/OpenAI/Groq at `backend/app/transcript_processor.py`, and an `ollama` client at L120/258 with an `OLLAMA_HOST` env-var-controlled destination) — same optional-cloud-LLM pattern as the Rust side. It is not started by the Tauri app by default; it's a separate Docker-based component.

## 8. Static/documentation links (system-browser only, not app HTTP calls)

| # | File:Line | Destination | Trigger | Class |
|---|---|---|---|---|
|25|`frontend/src/components/About.tsx:26`|`https://meetily.zackriya.com/#about`|"About" link → `open_external_url` (OS browser)|OPTIONAL_EXTERNAL|
|26|`frontend/src/components/AnalyticsConsentSwitch.tsx:150`|GitHub `PRIVACY_POLICY.md`|"View Privacy Policy" click|OPTIONAL_EXTERNAL|
|27|`frontend/src/components/ModelSettingsModal.tsx:739,1236`|`https://ollama.com/download`|"Download Ollama" prompt click|OPTIONAL_EXTERNAL|
|28|`frontend/src/components/BluetoothPlaybackWarning.tsx:84`|GitHub doc link (plain `<a href>`)|Click|OPTIONAL_EXTERNAL|
|29|`frontend/src/components/onboarding/steps/SetupOverviewStep.tsx:101`|GitHub repo link|Click|OPTIONAL_EXTERNAL|

## Dead code (not compiled — not a live call site, but should be removed)

- `frontend/src-tauri/src/lib_old_complex.rs` — contains loopback/example URL literals in test-like code; not referenced by any `mod` declaration, confirmed unreferenced, does not compile into the binary.
- `frontend/src-tauri/src/audio/core-old.rs` — same.

## Build-time only (not shipped app behavior)

- `.github/workflows/*.yml` — fetches Vulkan SDK during CI builds only.
- `scripts/generate-update-manifest-github.js`, `scripts/test-update-locally.js` — maintainer release tooling, never run by the shipped app.
- `frontend/src-tauri/build/onnxruntime.rs`, `build/ffmpeg.rs` — build-script downloads of ONNXRuntime/ffmpeg at compile time, not runtime calls.

## Open items for Phase 3 (offline architecture)

1. Decide whether to keep or gate the 4 cloud LLM providers (OpenAI/Groq/OpenRouter/Anthropic) behind a stricter "strict offline mode" switch that disables them entirely.
2. Remove the dormant `https://api.ollama.ai` CSP allowance (§3 note) — no code path uses it.
3. Harden `backend/app/main.py:646` to bind `127.0.0.1` instead of `0.0.0.0` by default.
4. Confirm ownership/trust of `meetily.towardsgeneralintelligence.com` (Parakeet v3 model host) before treating it as equivalent-trust to HuggingFace.
5. Decide whether the updater should keep pointing at the upstream Meetily GitHub releases or move to a Noetis-owned release feed once one exists.
6. Delete dead files `lib_old_complex.rs` and `audio/core-old.rs` (housekeeping, zero risk).
