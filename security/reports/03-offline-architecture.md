# Phase 3 — Local-Only / Offline Architecture Review

Static review only, baseline `a2cb62e`, branch `security-hardening`. No code executed or modified. Builds on [01-inventory.md](01-inventory.md) and [02-network-audit.md](02-network-audit.md) — not re-derived here. This is a **review/recommendation** report; nothing has been disabled or changed.

## 1. Per-integration: is there already a way to fully disable it?

| Integration | Existing disable mechanism | Status |
|---|---|---|
| OpenAI / Groq / OpenRouter / Anthropic (summary LLM) | `settings.provider` is a single-select column (`migrations/20250916100000_initial_schema.sql`). Selecting a different provider in `frontend/src/components/ModelSettingsModal.tsx:34,236-240` routes summary generation to that provider only (`frontend/src-tauri/src/summary/llm_client.rs:301-330`). | Already exists, but only as "one active provider at a time" — not a disable-all switch (see §2). |
| API keys for non-active providers | Stored independently per-provider: `openrouterApiKey`, `geminiApiKey`, `customOpenAIConfig` (various migrations), retrieved via `api_get_api_key` (`ModelSettingsModal.tsx:206-216`). | No UI "forget all keys"/"purge cloud credentials" action exists. Would need to be built for strict offline mode. |
| Ollama (remote endpoint) | Endpoint is a user-editable text field, format-validated only (see §3). | Not enforced as local-only. |
| PostHog telemetry | `AnalyticsConsentSwitch.tsx` opt-in toggle, `AnalyticsProvider.tsx:23-55` defaults `analyticsOptedIn=false`, one-time migration (`analyticsDefaultOffMigrationV1`) forces existing installs off too. | Already exists and works — default-off, explicit opt-in. |
| Model downloads (Whisper/Parakeet/Built-in-AI) | Only happen on explicit user action; no background/automatic download path found. | Already effectively opt-in by design. |
| Auto-updater | `UpdateCheckProvider.tsx:25-27` sets `checkOnMount: true`; `useUpdateCheck.ts:49-58` fires a GitHub Releases check ~2s after every app mount, unconditionally rendered in `app/layout.tsx:242-270`. No toggle found in `PreferenceSettings.tsx` or `BetaSettings.tsx`. | **Does not exist** — no setting anywhere disables the startup update check. |

## 2. Global "strict offline mode" flag

Repo-wide case-insensitive grep for `offline`, `strict.?mode`, `airplane` — all hits are incidental UI copy (connection-loss handling, onboarding text) or unrelated variable names. **No global offline/strict/airplane-mode flag exists anywhere in code, DB schema, or Tauri config.** This must be built from scratch — there is no partial implementation to extend.

**Concrete gap**: nothing stops a user from simultaneously having `provider = "ollama"` *and* a saved Anthropic/OpenAI/Groq/OpenRouter API key sitting in the `settings` table (never cleared on provider switch) *and* those other providers' live model-list fetches firing the instant their Settings tab is opened (`ModelSettingsModal.tsx:257-300`, backed by `anthropic.rs:103`, `openai.rs:130`, `groq.rs:98`, `openrouter.rs:45` — all fire on tab-open/mount, independent of the active provider). A user nominally "on Ollama" can still trigger outbound cloud calls with no warning. No code currently prevents this dual state.

## 3. Ollama endpoint — restricted to loopback?

`frontend/src-tauri/src/ollama/ollama.rs`:
- `validate_endpoint_url()` (`ollama.rs:80-94`) only checks the string starts with `http://` or `https://`. **No host/scheme allowlist, no loopback restriction.**
- `is_localhost_endpoint()` (`ollama.rs:68-78`) substring-matches `"localhost"`, `"127.0.0.1"`, `"::1"` — but is **not used to block anything**; it only decides whether to fall back to the local `ollama` CLI on an HTTP error (`ollama.rs:117-120`). It has zero bearing on what URL the app is allowed to call.
- Outbound calls (`ollama.rs:162,281,440`, `llm_client.rs:314-322`) POST/GET to whatever URL is in `settings.ollamaEndpoint` — fully attacker-controllable if tampered with or a user is socially engineered into pasting a malicious "Ollama-compatible" endpoint.

**Confirmed finding**: a "local" Ollama setting can silently point at arbitrary remote infrastructure — no enforcement exists today. A real fix needs actual DNS resolution + IP-range checking at request time (string matching alone doesn't defend against DNS rebinding), which does not currently exist and would need to be built.

## 4. Local service bind addresses

| Service | Bind call | File:Line | Address |
|---|---|---|---|
| Python/FastAPI legacy backend (port 5167) | `uvicorn.run(...)` | `backend/app/main.py:646` | **`0.0.0.0`** — confirmed, matches Phase 2 flag |
| Legacy whisper-server, Windows launch scripts | `--host 127.0.0.1 --port 8178` | `backend/clean_start_backend.cmd:161`, `backend/start_whisper_server.cmd:39,46` | `127.0.0.1` |
| Legacy whisper-server, Bash/Docker path | `--host "0.0.0.0"` hardcoded | `backend/clean_start_backend.sh:248` | **`0.0.0.0`** |
| Legacy whisper-server, Docker entrypoint | `WHISPER_HOST=${WHISPER_HOST:-0.0.0.0}` | `backend/docker/entrypoint.sh:36,331` | **`0.0.0.0`** default |
| Rust/Tauri app itself | No server-bind code found | — | Does not run its own backend/whisper-stream server. `api.rs:20` (`APP_SERVER_URL`) and the dead `lib_old_complex.rs:72` are client-side constants presupposing the legacy Python backend/whisper-server runs as a separate process. All `TcpListener::bind("127.0.0.1:0")` hits elsewhere are ephemeral test-only listeners inside `#[cfg(test)]` blocks. |

**Net finding**: the `0.0.0.0` binding is real but only matters if the "archived/unsupported" Python backend (per `CLAUDE.md:5-10`, itself untrusted repo text, but consistent with what the code shows) is run standalone/via Docker — off by default for the shipped Tauri app. No `externalBin`/sidecar wiring to the Python backend exists in `tauri.conf.json` (`externalBin` only lists `binaries/llama-helper`, `binaries/ffmpeg`), supporting "not bundled/not auto-started" — but **not runtime-verified; needs Phase 10 confirmation.**

## 5. Centralized outbound-authorization choke point

**Does not exist today.** Every integration module builds and owns an independent `reqwest::Client`:

- `groq.rs:95`, `anthropic.rs:100`, `openai.rs:127`, `ollama.rs:162,281,440` (3 separate instantiations in one file), `llm_client.rs:698,784,860`, `api.rs:247,1085,1127,1306`, `whisper_engine.rs:1143`, `parakeet_engine.rs:865`, `summary_engine/model_manager.rs:473`.
- Analytics uses the `posthog-rs` crate's own internal HTTP client — a third, independent transport path.

No `tauri-plugin-http` (Tauri's URL-scoped HTTP allowlist plugin) is used, and no shared `http_client.rs`/`network.rs` wrapper exists. **There is no single point today through which an offline-mode flag could transparently block all outbound calls.** Implementing strict offline mode requires either (a) a shared client factory all ~10 call sites are refactored to use, or (b) a flag check duplicated at each site (error-prone, easy to miss on new integrations).

## 6. OS-level guidance needed

App-level restriction — even a well-built strict-offline flag — cannot control a system-wide Ollama the user already runs as its own OS process, or any other local tool outside this app's process tree. For genuine network isolation, the user/deployer needs OS-level enforcement:

- **Windows Firewall** outbound rules scoping the Noetis executable (and `llama-helper.exe`/sidecars) to loopback-only, or blocking it except for explicitly allowed local ports (11434, 5167, 8178).
- Equivalent on macOS (Little Snitch / `pf`) or Linux (`iptables`/`nftables`).
- This should be documented as user guidance — the app has no built-in mechanism to enforce or verify OS firewall state.

## Summary classification

| # | Item | Status |
|---|---|---|
| 1 | Per-provider disable | Exists for LLM providers (single-select) and telemetry (opt-in); **no** disable for the updater; **no** key-purge action |
| 2 | Global strict-offline flag | **Does not exist** — must be built from scratch |
| 3 | Ollama endpoint restricted to loopback | **Does not exist** — validation is scheme-only, no host allowlist |
| 4 | Local service binds | `backend/app/main.py:646` = `0.0.0.0` (Docker/Bash path); Windows scripts = `127.0.0.1`; Tauri app itself starts no server — **needs Phase 10 runtime confirmation** |
| 5 | Centralized outbound gate | **Does not exist** — ~10 independent `reqwest` clients, no allowlist plugin in use |
| 6 | OS-level guidance | Not a code fix — document Windows Firewall / pf / iptables guidance |

## Recommendations for remediation (pending approval — not implemented)

1. Add a shared HTTP client factory / outbound gate that every integration module routes through, so a future strict-offline flag has one place to enforce from.
2. Add a real "Strict Offline Mode" setting that: disables the 4 cloud LLM providers outright (not just "not selected"), blocks the auto-update check, and restricts the Ollama/custom-OpenAI endpoint fields to a validated loopback allowlist (resolved-IP check, not substring match).
3. Add an explicit "forget all cloud API keys" action in Settings.
4. Add a disable toggle for the auto-update check.
5. Harden the legacy Python backend's default bind to `127.0.0.1` (low urgency since it's not bundled with the shipped app — confirm in Phase 10 before prioritizing).
