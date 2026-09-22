# Phase 1 — Repository Inventory

Static read-only review at baseline commit `a2cb62e` (see [00-baseline.md](00-baseline.md)). No scripts, installers, or builds were executed. Any instructions found inside repo files (README, `CLAUDE.md`, scripts) were treated as untrusted data, not commands.

## 1. Rust crates / Cargo workspace

Root `Cargo.toml:1-11` — workspace members: `frontend/src-tauri`, `llama-helper` (resolver "2", edition 2021, rust-version 1.77).

**`frontend/src-tauri/Cargo.toml`** (package `meetily` v0.4.1):
- Tauri 2.6.2 + plugins: `tauri-plugin-fs 2.4.0`, `tauri-plugin-dialog 2.3.0`, `tauri-plugin-store 2.4.0`, `tauri-plugin-notification 2.3.1`, `tauri-plugin-updater 2.3.0`, `tauri-plugin-process 2.3.0`, `tauri-plugin-single-instance =2.3.7`
- Transcription: `whisper-rs 0.13.2`, `ort 2.0.0-rc.10` (ONNX Runtime, for Parakeet)
- Audio: `cpal 0.15.3`, `nnnoiseless 0.5`, `ebur128 0.1`, `symphonia 0.5.4`, `rubato`, `ringbuf`
- DB: `sqlx 0.8` (sqlite, runtime-tokio, chrono)
- Networking/LLM clients: `reqwest 0.11`; custom modules for Anthropic/OpenAI/Groq/Ollama (`src/anthropic`, `src/openai`, `src/groq`, `src/ollama`)
- **Git dependencies** (`Cargo.toml:104,146,173`): `silero_rs` → `github.com/emotechlab/silero-rs, rev=26a6460`; `ffmpeg-sidecar` → `github.com/nathanbabcock/ffmpeg-sidecar, branch=main` (**unpinned branch**, not a rev/tag); `cidre` (macOS only) → `github.com/yury/cidre, rev=a9587fa`
- **`[patch.crates-io]`** (`Cargo.toml:212-214`): overrides `cpal` → git fork `RustAudio/cpal@51c3b43`, `esaxx-rs` → git fork `thewh1teagle/esaxx-rs@feat/dynamic-msvc-link`
  - **Anomaly**: `Cargo.lock` does *not* reflect these patches — `cpal` (L1104-1106) and `esaxx-rs` (L1944-1946) both resolve to plain crates.io registry sources with checksums, not the declared git forks. Only `cidre`, `ffmpeg-sidecar`, and `silero-rs` actually show `source = "git+..."` in the lockfile. Either the lockfile is stale or the patches aren't taking effect — **needs human confirmation of which dependency code is actually built.**

**`llama-helper/Cargo.toml`**: standalone crate, `llama-cpp-2 = "=0.1.146"` (pinned exact), `anyhow`, `serde`, `serde_json`, `encoding_rs`; release profile `lto=true`, `opt-level="s"`.

## 2. Frontend (Next.js / React)

`frontend/package.json`: Next.js `^14.2.25`, React `^18.2.0`, `@tauri-apps/api ^2.6.0` + matching plugins (fs, notification, os, process, store, updater), TipTap/BlockNote/Remirror editors, Radix UI, Tailwind, Zod, react-hook-form.
- Lockfile: `pnpm-lock.yaml` (pnpm is the package manager).
- **Anomaly**: `package.json:5` declares `"main": "electron/main.js"` but no `frontend/electron/` directory exists — leftover from an earlier Electron iteration (project is now Tauri-only). Dead field, cleanup candidate.
- No `postinstall`/`preinstall`/`prepare`/`predev`/`prebuild` lifecycle hooks anywhere in the repo (grepped all `package.json` files, zero matches).

## 3. Python components

`backend/` — Python/FastAPI backend (`main.py`, `db.py`, `transcript_processor.py`, `schema_validator.py`), `backend/requirements.txt`: `fastapi==0.115.9`, `uvicorn==0.34.0`, `pydantic-ai==0.2.15`, `pydantic==2.11.5`, `pandas==2.2.3`, `python-multipart==0.0.20`, `aiosqlite==0.21.0`, `ollama==0.5.2`, `python-dotenv==1.1.0`, `devtools==0.12.2`.

**Important**: `CLAUDE.md:5-10` (repo's own AI-guidance doc, read as data) states this Python/FastAPI backend, Docker setup, and standalone whisper-server are "archived and unsupported" — the actively supported app is the Tauri/Rust desktop app. **A human should verify this claim** before deciding how much audit weight the Python backend deserves.

Other Python: `backend/debug_cors.py`, `backend/examples/run_summary_workflow.py`, `scripts/inject_transcript.py` (standalone SQLite transcript-injection utility).

## 4. Tauri integration

`frontend/src-tauri/tauri.conf.json`:
- `identifier`: `com.meetily.ai`; bundle targets: deb, appimage, msi, nsis, app, dmg
- **CSP** (`:26-31`): `default-src 'self'`; `img-src 'self' asset: https://asset.localhost data:`; `connect-src 'self' http://localhost:11434 http://localhost:5167 http://localhost:8178 https://api.ollama.ai`. Permits plaintext `http://` to three local ports plus one remote HTTPS host. No cloud-LLM domains present — those calls happen from Rust, not webview JS, so not necessarily a gap.
- **Capabilities** (`:38-77`, single "main" capability): broad filesystem grants — `fs:default`, `fs:read-all`, `fs:write-all`, `fs:allow-app-read/write`, `fs:allow-download-read/write`, scoped `fs:scope` to `$APPDATA/*`.
- **Asset protocol**: enabled, scoped to `$APPDATA/**`.
- **Updater** (`:113-120`): minisign pubkey embedded, endpoint `https://github.com/Zackriya-Solutions/meeting-minutes/releases/latest/download/latest.json` — **points at the upstream repo, not `binesh-balan/noetis`.** Confirm intentional (fork still trusts upstream's release feed/signing key) or needs repointing.
- Windows signing (`:109-111`): `scripts/sign-windows.ps1` no-ops unless `$env:DIGICERT_KEYPAIR_ALIAS` is set.
- macOS: `entitlements.plist`, `hardenedRuntime: true`, `signingIdentity: "-"` (ad-hoc).

## 5. Native DLL/dylib/.so and vendored native deps

- **`build.rs` downloads native binaries at compile time** (`frontend/src-tauri/build.rs:1-25`):
  - `build/ffmpeg.rs`: downloads platform FFmpeg archives from `github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/...`. **No checksum/hash verification** — only a functional smoke test (`ffmpeg -version`). Zip extraction guards against Zip-Slip via `enclosed_name()`.
  - `build/onnxruntime.rs`: downloads `microsoft/onnxruntime` v1.22.0, Windows/x64 only. **Does verify SHA-256 + exact byte size** per artifact, including reparse-point/symlink checks — notably more hardened than the FFmpeg path.
  - **Inconsistency flagged**: FFmpeg download lacks the integrity verification the ONNX Runtime download has, in the same build pipeline.
- `tauri.conf.json:100-103` `externalBin`: `binaries/llama-helper`, `binaries/ffmpeg` (fetched at build time, gitignored — not committed).
- `backend/whisper-custom/server/httplib.h` — vendored cpp-httplib single header in the legacy Python backend's custom whisper server.
- `backend/whisper.cpp` — git submodule, present but **not initialized** in this checkout (see §9).
- `frontend/src-tauri/check_screen_permission.swift` — macOS Swift helper for screen-recording permission checks.

## 6. Shell / PowerShell / batch scripts

| Path | Purpose |
|---|---|
| `backend/build-docker.ps1` / `.sh` | Build backend Docker images |
| `backend/build_whisper.cmd` / `.sh` | Build whisper.cpp/whisper-server binary |
| `backend/clean_start_backend.cmd` / `.sh` | Clean-rebuild and start Python backend |
| `backend/docker/entrypoint.sh` | Docker container entrypoint |
| `backend/download-ggml-model.cmd` / `.sh` | Download ggml Whisper models from HuggingFace |
| `backend/install_dependancies_for_windows.ps1` | Install Windows build prerequisites |
| `backend/run-docker.ps1` / `.sh` | Run backend Docker Compose stack |
| `backend/set_env.sh` | Interactively populate `backend/.env` (API keys) |
| `backend/setup-db.ps1` / `.sh` | Initialize backend SQLite schema |
| `backend/start_python_backend.cmd` | Launch FastAPI backend directly |
| `backend/start_whisper_server.cmd` | Launch standalone whisper server |
| `backend/start_with_output.ps1` | Start backend with logging captured |
| `frontend/build.bat` / `.ps1` / `build_backup.bat` | Windows Tauri build |
| `frontend/build-gpu.bat` / `.ps1` / `.sh` | GPU-enabled Tauri build |
| `frontend/clean_build.sh` / `clean_build_windows.bat` | Clean build artifacts + rebuild |
| `frontend/clean_run.sh` / `clean_run_windows.bat` | Clean dev run |
| `frontend/dev-gpu.bat` / `.ps1` / `.sh` | Dev server with GPU detection |
| `frontend/package-app.sh` | Package the built app |
| `frontend/scripts/load-env.ps1` | Parse `.env` into current PowerShell session |
| `frontend/src-tauri/scripts/sign-windows.ps1` | Windows code-signing wrapper (DigiCert), no-op unless configured |

## 7. Build scripts / lifecycle hooks

- `build.rs` (see §5) — downloads FFmpeg + ONNX Runtime at build time, prints GPU guidance, calls `tauri_build::build()`.
- No `postinstall`/`preinstall`/`prepare`/`predev`/`prebuild` scripts anywhere (confirmed via grep, zero matches).

## 8. GitHub Actions workflows

| Workflow | Trigger(s) | Permissions |
|---|---|---|
| `build.yml` | `workflow_call` (reusable) | `contents: write` |
| `build-windows.yml` | `workflow_dispatch` | `contents: write` |
| `build-macos.yml` | `workflow_dispatch` | `contents: write` |
| `build-linux.yml` | `workflow_dispatch` | `contents: write` |
| `build-devtest.yml` | `workflow_dispatch` | `contents: write` |
| `build-test.yml` | `workflow_dispatch` | `contents: write` |
| `release.yml` | `workflow_dispatch` | `contents: write` (3 jobs) |
| `pr-main-check.yml` | `workflow_dispatch` | none declared |

- **No workflow triggers on `pull_request`/`pull_request_target`** — all manual or reusable, so untrusted-PR-triggered Actions privilege escalation doesn't apply here.
- All `contents: write` grants are scoped to manually/maintainer-triggered builds, not automatic on external contributions.
- **Third-party actions, none pinned to a commit SHA** (all on tag/major-version refs, which are mutable): `actions/checkout@v4`, `actions/setup-node@v4`, `actions/cache@v4`, `actions/upload-artifact@v4`, `actions/github-script@v7`, `dtolnay/rust-toolchain@stable` (floating, not even a version tag), `swatinem/rust-cache@v2`, `pnpm/action-setup@v4`, `humbletim/install-vulkan-sdk@v1.2`, `digicert/ssm-code-signing@v1.1.1`, `tauri-apps/tauri-action@v0`. Recommend pinning at least the lesser-known ones (`humbletim/install-vulkan-sdk`, `digicert/ssm-code-signing`) to commit SHAs.
- `.github/force-portable-ggml.cmake`, `.github/verify-portable-ggml.cjs`(+test) — CI verification tooling, not binaries.

## 9. Git submodules and Git LFS

- `.gitmodules`: one submodule, `backend/whisper.cpp` → `github.com/Zackriya-Solutions/whisper.cpp`, branch `develop`. **Not checked out** in this clone (uninitialized).
- **No Git LFS** — no `.gitattributes`, no `*.lfs` pointers. Large binaries under `docs/` (e.g. `docs/meetily-export.gif` 15.5MB, `docs/meetily_demo.gif` 9MB) are stored as regular git blobs.

## 10. Precompiled binaries / embedded executables tracked in git

`git ls-files` filtered to `.exe/.dll/.so/.dylib/.bin` returns exactly one hit:

- **`frontend/vs_buildtools.exe` — 4,460,128 bytes, tracked in git.** Present since commit `dd5401d` ("Initial parkeet test") through HEAD — long-standing, not a recent addition. **Genuine finding**: a committed Windows executable (apparently the Visual Studio Build Tools bootstrapper) sitting in `frontend/`. Needs provenance/hash verification against the official Microsoft installer, and a decision on whether it should be fetched at build/CI time instead (like ffmpeg/onnxruntime) rather than committed.
- No other binaries tracked — `.gitignore` entries (`backend/.gitignore:17` ignores `*.bin`, `frontend/.gitignore:6` ignores `**/models/*.bin`, root `.gitignore:79` ignores `frontend/src-tauri/binaries/`) are consistent with binaries being fetched/built, except for the one exception above.

## 11. Model files and download mechanisms

- **Whisper (ggml)**: hardcoded URL map in `whisper_engine.rs:1076-1089`, all → `huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-*.bin`. Verifies minimum file size post-download; unit-tested for user-agent/undersized-file rejection.
- Legacy Python backend has an equivalent downloader (`backend/download-ggml-model.sh`/`.cmd`), same HuggingFace source.
- **Parakeet (ONNX)**: `parakeet_engine.rs:120-139` — v3 from `meetily.towardsgeneralintelligence.com/models/...` (project-operated, non-HuggingFace domain — **confirm ownership/TLS posture**); v2 from HuggingFace, pinned to a specific commit hash. Both declare exact expected byte sizes for verification.
- **Ollama**: local HTTP client only, no model-download logic in-app — pulls delegated to the user's local Ollama install.
- Cloud LLM model-listing (not weight downloads) for OpenAI/Anthropic/Groq use user-supplied API keys stored via app settings/DB (see §13).

## 12. Installer / updater components

- Tauri bundler targets `deb`, `appimage`, `msi`, `nsis`, `app`, `dmg` (generated by Tauri CLI at build time; no custom WiX/NSIS scripts committed).
- Updater: `tauri-plugin-updater 2.3.0` + minisign pubkey, endpoint on upstream `Zackriya-Solutions/meeting-minutes` releases (see §4).
- `scripts/generate-update-manifest-github.js` — generates `latest.json` update manifest from build artifacts + GitHub Release URLs.
- `scripts/test-update-locally.js` — local HTTP server serving `latest.json` to test the OTA flow pre-publish.
- `frontend/src-tauri/scripts/sign-windows.ps1` — Windows signing wrapper for the Tauri bundler's `signCommand`.

## 13. Env vars and config files

- `backend/temp.env` — **placeholder values only**: `ANTHROPIC_API_KEY=api_key_here`, `GROQ_API_KEY=gapi_key_here`, `OPENAI_API_KEY=api_key_here`. No real secrets.
- `backend/set_env.sh` — interactively writes those three keys into a generated, git-ignored `backend/.env` (not present in this checkout).
- `frontend/.env.example` — documents `TAURI_SIGNING_PRIVATE_KEY`/`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (blank).
- `frontend/src-tauri/config/backend_config.json` — non-secret local endpoints: `whisperEndpoint: http://127.0.0.1:8178/stream`, `ollamaEndpoint: http://localhost:11434`, `fastApiEndpoint: http://localhost:5167`.
- DB migrations adding API-key columns (schema only): `migrations/20250920155811_add_openrouter_api_key.sql`, `20251105120000_add_pro_license_custom_openai.sql`, `20251229000000_add_gemini_api_key.sql`, `20251010153942_add_ollama_endpoint.sql` — **whether these keys are encrypted at rest in the local SQLite DB or stored plaintext needs checking in Phase 8 (data protection).**
- `frontend/src-tauri/.cargo/config.toml` — macOS deployment-target rustflags only, no secrets.
- **Secret-pattern grep** (`sk-ant-…`, `sk-…`, `AKIA…`, `-----BEGIN…PRIVATE KEY-----`, `ghp_…`, `xox[baprs]-…`): **zero matches** in the working tree. Broader `api_key|secret|token` grep hit 53 files, all legitimate application code/migrations/tests referencing the *concept* of API keys, not hardcoded values.

## 14. Git history review

`git log --oneline -50` shows normal iterative history on `security-hardening` (branched from `a2cb62e`). `git log --diff-filter=A --stat -30` (added files, last 30 commits) — no suspicious binary substitutions. Notable additions: `docs/meetily-export.gif` (15.5MB demo asset, plausible), `frontend/src-tauri/tests/fixtures/he_aac_48k_5s.m4a` (32KB test fixture, plausible), `Cargo.lock` and `frontend/pnpm-lock.yaml` both added to version control partway through history rather than from project start (not inherently suspicious, but dependency-pinning discipline only began at those commits). No `.exe/.dll/.so` additions in the last 30 commits — the one tracked `.exe` (`vs_buildtools.exe`, §10) predates this window.

---

## Anomalies flagged for human review (not asserted as malicious)

1. `Cargo.lock` doesn't reflect the `cpal`/`esaxx-rs` git-fork patches declared in `Cargo.toml` — confirm which dependency code is actually built (§1).
2. `frontend/vs_buildtools.exe` — 4.46MB committed Windows executable, long-standing in history (§10).
3. FFmpeg build-time download has no checksum verification, unlike the ONNX Runtime download in the same pipeline (§5).
4. Updater endpoint and `Cargo.toml` `repository` field still point at upstream `Zackriya-Solutions/meeting-minutes` rather than `binesh-balan/noetis` (§4) — confirm intended or repoint.
5. Parakeet v3 model host `meetily.towardsgeneralintelligence.com` is a non-GitHub/non-HuggingFace third-party-looking domain (§11) — confirm ownership/TLS posture.
6. `frontend/package.json` still declares `"main": "electron/main.js"` with no `electron/` directory present — stale, cleanup only (§2).
7. GitHub Actions third-party actions pinned to tags/major versions, not commit SHAs (§8) — hardening opportunity, especially `humbletim/install-vulkan-sdk`, `digicert/ssm-code-signing`.
8. API keys stored via DB migrations (§13) — encryption-at-rest status unconfirmed, needs Phase 8 follow-up.
9. `ffmpeg-sidecar` git dependency tracks a branch (`main`), not a pinned rev/tag (§1) — supply-chain hygiene issue, should pin to a commit.
