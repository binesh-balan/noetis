# Remediation Log

All 18 findings from the Phase 1-9 static audit (`security/reports/`) have a remediation entry below — the 2 HIGH findings the repo owner prioritized first, and the 16 MEDIUM/LOW findings fixed in the same session after "fix all." **None of this has been compiled.** `cargo`/`rustc` are not installed in the environment this work was done in; every Rust change was written and manually traced against known-stable APIs and this codebase's own existing patterns, but a `cargo check`/build pass is required before merging. Frontend (TypeScript) changes were verified with `npx tsc --noEmit` (zero errors) since a Node toolchain was available. Two workflow YAML files and one new workflow were validated as syntactically correct YAML via `python3 -c "import yaml"`.

Phase 10 (build in an isolated environment, run with synthetic audio, capture network traffic) was explicitly deferred by the repo owner and was not run.

---

## HIGH severity

### 1. Plaintext API keys, ungated IPC retrieval

- **CWE**: CWE-312 (Cleartext Storage) + CWE-306 (Missing Authentication, partially addressed)
- **Evidence**: `security/reports/05-rust-security.md` §13,17
- **Fix**: `frontend/src-tauri/src/secure_storage.rs` — Windows DPAPI encryption at rest (`CryptProtectData`/`CryptUnprotectData`, zero new dependencies), wired into every API-key read/write path in `setting.rs` including the `customOpenAIConfig` JSON blob. Legacy plaintext values auto-migrate to protected form on next save. Documented (not silently patched) the accepted webview↔backend IPC trust boundary on `api_get_api_key`/`api_get_transcript_api_key`, since building real session-token auth blind, unverifiable by a compiler, was judged too risky.
- **Residual**: macOS/Linux still plaintext (Windows-only DPAPI). IPC retrieval trust boundary is documented, not eliminated — a webview compromise still gets the decrypted key back.

### 2. Deleted meetings leave audio/transcript files on disk

- **CWE**: CWE-459 (Incomplete Cleanup)
- **Evidence**: `security/reports/08-data-protection.md` §14
- **Fix**: `database/repositories/meeting.rs`'s `delete_meeting` now returns the meeting's `folder_path`; `api_delete_meeting` removes that on-disk folder after a successful DB transaction (best-effort, logged not fatal on failure).
- **Residual**: `meeting_notes` may still orphan (foreign_keys pragma never enabled). SQLite WAL/journal may retain deleted bytes until `VACUUM` — outside what application code can fully guarantee.

---

## MEDIUM/LOW severity (fixed in the "fix all" pass)

| # | Finding | Fix | Files |
|---|---|---|---|
| 4 | `open_external_url` handed unvalidated URL to `cmd /C start` | Reject non-http(s) URLs; replaced with `rundll32 url.dll,FileProtocolHandler` (no shell involved) | `api/api.rs` |
| 5 | FFmpeg downloads unverified; CWD-relative binary lookup | Pinned SHA-256 (from GitHub's own release-asset digests) for build-time FFmpeg archive; removed the CWD fallback from runtime ffmpeg resolution | `build/ffmpeg.rs`, `audio/ffmpeg.rs` |
| 6 | CI leaked `SM_API_KEY` prefix to logs; no post-build installer verification | Replaced `.Substring(0,20)` with `SET`/`NOT SET`; ported `codesign --verify`/`Get-AuthenticodeSignature` checks into the actual release-build workflow | `.github/workflows/build.yml`, `build-windows.yml` |
| 7 | `Cargo.lock`/`[patch.crates-io]` drift for cpal/esaxx-rs | Documented in-place with a comment explaining the discrepancy and what `cargo update`/`cargo tree` to run — hand-editing a machine-generated lockfile without a compiler was judged too risky | `Cargo.toml` |
| 8 | Committed unused `vs_buildtools.exe`; repo metadata pointed at upstream | Deleted the binary (confirmed unreferenced); updated `Cargo.toml` `repository` field to this fork. Updater endpoint left alone — repointing it needs a real minisign keypair + hosted release, not a code edit | `Cargo.toml`, file deletion |
| 9 | `fs:read-all`/`fs:write-all` capability unused but granted | Removed all `fs:*` permissions from the main window capability (confirmed zero webview JS uses `@tauri-apps/plugin-fs`); also dropped the dormant `api.ollama.ai` CSP entry | `tauri.conf.json` |
| 10 | No SAST/audit/secret-scan tooling in CI; no SBOM | New `.github/workflows/security-scan.yml`: cargo fmt/clippy/audit, pnpm audit, gitleaks, SBOM via syft — manual+weekly trigger, all checks non-blocking until a human reviews a first baseline | `.github/workflows/security-scan.yml` |
| 11 | No cryptographic hash verification on any model download | Pinned SHA-256 (fetched from each publisher's own git-lfs pointer metadata, not self-computed) for 12 Whisper models, 4 Parakeet v2 artifacts, 4 on-device GGUF models. Parakeet v3 (non-HuggingFace host) stays size-only — 2 of its 4 files return non-reproducible multipart-upload ETags, and hashing the other 2 would mean downloading ~670MB blind | `whisper_engine.rs`, `parakeet_engine.rs`, `summary_engine/{models,model_manager}.rs`, `Cargo.toml` (promoted `sha2` to a regular dependency) |
| 12 | Prompt-injection defense only in final-report stage | Added the same "ignore embedded instructions" rule to the per-chunk and combine-stage system prompts | `summary/processor.rs` |
| 13 | No license disclosure for Qwen/Gemma on-device models | Added to README Acknowledgments; added a visible note + link in the Built-in AI model picker UI | `README.md`, `BuiltInModelManager.tsx` |
| 14 | Dead code held most of the tree's raw unsafe/static-mut | Deleted `lib_old_complex.rs`, `audio/core-old.rs`, `audio/recording_saver_old.rs`, `audio_v2/` (confirmed unreferenced by any `mod` declaration) | file deletions |
| 15 | `static mut SAMPLE_COUNTER` — unsynchronized global state | Replaced with `AtomicU64` | `audio/pipeline.rs` |
| 16 | Fragile `format!`-built SQL column names | Consolidated 5 duplicated provider→column `match` blocks into 2 shared helper functions | `database/repositories/setting.rs` |
| 17 | Export used a Blob download to a fixed location | New `export_text_content` command opens a native save dialog (`tauri-plugin-dialog`, mirroring the existing `select_legacy_database_path` pattern) and writes server-side | `api/api.rs`, `AISummary/index.tsx` |
| 18 | No restrictive permissions on app-created directories | Meeting recording folders (and `.checkpoints/`) restricted to owner-only (0700) on Unix after creation; no-op on Windows, which already inherits a user-scoped ACL | `audio/audio_processing.rs` |
| 19 | Dead `dangerouslySetInnerHTML` demo page | Deleted `app/notes/[id]/page.tsx`. Correction to the original finding: `Sidebar/index.tsx:582` does link to this route for non-hyphenated IDs, but the rendered content was always one of 4 hardcoded sample strings, never derived from real data — the "unreachable" framing was imprecise, not the security conclusion | file deletion |
| 3 | No strict offline mode, no centralized outbound gate, no real Ollama loopback check | New `settings.strictOfflineMode` flag + `network_policy.rs` (in-memory cache, synced at startup and on change) gates `llm_client.rs`'s `generate_summary` — the single choke point every cloud/local provider call already funnels through. Real DNS-resolution-based loopback check (`ollama::resolve_to_loopback_only`) replaces the substring check for the Ollama endpoint when strict mode is on. Update checker skips itself when strict mode is on. Added a "Forget All API Keys" action. See §"Residual" below — this is the largest, most partial fix in this pass | `network_policy.rs` (new), `llm_client.rs`, `ollama.rs`, `setting.rs`, `api.rs`, `lib.rs`, migration, `useUpdateCheck.ts`, `PreferenceSettings.tsx` |

### #3 residual scope (explicitly not covered)

Strict Offline Mode gates the actual summarization call path (`generate_summary`) and the update checker — the two places Phase 3 identified as carrying real content or being unconditional. It does **not** gate the other ~9 independent `reqwest::Client` instantiations Phase 3 catalogued (model-listing calls for each cloud provider when a user opens that Settings tab, model downloads, analytics). Opening the OpenAI tab in Model Settings will still fire a request even with strict mode on. A full centralized HTTP client factory across every integration module remains a larger follow-up — Phase 3's own report frames this as the "right" fix but a bigger refactor than was safe to do blind in this session.
