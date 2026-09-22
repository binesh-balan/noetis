# Remediation Log

Two findings were remediated on `security-hardening` (commit `4e3782e`, on top of baseline `a2cb62e`). Both were selected by the repo owner as highest-priority after reviewing the Phase 1-9 static audit (`security/reports/`). Phase 10 (runtime network validation) was explicitly deferred by the owner and has not run; neither has a compiler, since `cargo`/`rustc` are not installed in the audit environment — **these changes have not been compiled or tested.** A `cargo check`/build pass is required before merging.

---

## Finding 1 — Plaintext API keys, ungated IPC retrieval

- **Severity**: HIGH
- **CWE**: CWE-312 (Cleartext Storage of Sensitive Information) + CWE-306 (Missing Authentication for Critical Function, partially addressed — see Residual Risk)
- **Affected files**: `frontend/src-tauri/src/database/repositories/setting.rs` (`save_api_key`, `get_api_key`, `save_transcript_api_key`, `get_transcript_api_key`, `save_custom_openai_config`, `get_custom_openai_config`); `frontend/src-tauri/src/api/api.rs` (`api_get_api_key`, `api_get_transcript_api_key`)
- **Evidence**: `security/reports/05-rust-security.md` §13, §17; `security/reports/01-inventory.md` §13
- **Exploitability**: Any process or tool with read access to the app's SQLite data file (malware, a backup/sync tool, another local user with filesystem access on a misconfigured system) could read every stored provider API key in plaintext. Separately, any JS running in the Tauri webview (via XSS, a compromised npm dependency, or a malicious imported file that triggers a rendering bug) could call `invoke('api_get_api_key', {provider})` and get the same plaintext key back, since the `_auth_token` parameter on that command was accepted but never validated.
- **Impact**: Full compromise of every cloud LLM provider credential the user has configured (OpenAI, Anthropic, Groq, OpenRouter, custom-OpenAI-compatible endpoints).
- **Remediation applied**:
  1. Added `frontend/src-tauri/src/secure_storage.rs` — encrypts API keys at rest using the Windows Data Protection API (`CryptProtectData`/`CryptUnprotectData`), bound to the current Windows user account. No new Cargo dependency was added (avoids an unverifiable dependency-resolution risk in an environment with no compiler available to check it). All API-key read/write paths in `setting.rs` (including the API key embedded in the `customOpenAIConfig` JSON blob) now route through `protect()`/`unprotect()`. Legacy plaintext values are transparently upgraded to protected form the next time they're saved — no separate migration step, no data loss for existing users.
  2. Documented (did not silently "fix" via unverified new auth plumbing) the accepted trust model on `api_get_api_key`/`api_get_transcript_api_key`: the Tauri webview and Rust backend are one trust domain by design in this app, matching Tauri's own security model (its capability/ACL system governs plugin commands, not custom `#[tauri::command]` functions). Building a real per-session credential system to gate IPC calls was judged too large and too unverifiable a change to make blind in an environment with no compiler.
- **Validation**: **Not compiled.** Three unit tests were added in `secure_storage.rs` (`protect_unprotect_roundtrip`, `unprotect_passes_through_legacy_plaintext`, `empty_string_roundtrips`) but have not been run. Needs `cargo test -p meetily secure_storage` (or equivalent) before merge, plus a manual check that existing saved API keys still work after upgrade (save→restart→retrieve roundtrip).
- **Residual risk**: See `RESIDUAL_RISKS.md` — macOS/Linux still store keys in plaintext (no OS keychain integration on those platforms yet); the IPC-retrieval trust-boundary question is documented, not eliminated; compilation is unverified.

## Finding 2 — Deleted meetings leave audio/transcript files on disk

- **Severity**: HIGH (data-retention/privacy gap — directly undercuts the app's stated privacy-first mission)
- **CWE**: CWE-459 (Incomplete Cleanup)
- **Affected files**: `frontend/src-tauri/src/database/repositories/meeting.rs` (`delete_meeting`, `delete_meeting_with_transaction`); `frontend/src-tauri/src/api/api.rs` (`api_delete_meeting`)
- **Evidence**: `security/reports/08-data-protection.md` §14
- **Exploitability**: N/A (not an attacker-triggered vulnerability) — a correctness/data-retention defect. A user who deletes a meeting believing its recording and transcript are gone would find `audio.mp4`, `transcripts.json`, and `metadata.json` still present in the recordings folder indefinitely.
- **Impact**: Directly contradicts user expectations and the app's privacy-first positioning; sensitive meeting audio/transcript content the user explicitly asked to delete remains recoverable on disk.
- **Remediation applied**: `delete_meeting_with_transaction` now looks up the meeting's `folder_path` before deleting its DB rows and returns it via a new `DeletedMeeting` struct. `api_delete_meeting` removes that on-disk folder (`std::fs::remove_dir_all`) after the DB transaction commits successfully. Failure to remove the folder is logged as a warning, not treated as fatal — the already-committed DB deletion is not rolled back on a filesystem error (e.g. a file still open elsewhere), matching this codebase's existing best-effort-cleanup pattern used for checkpoint cleanup elsewhere.
- **Validation**: **Not compiled.** No automated test was added for this path (would need a DB+filesystem integration test harness this session didn't have time to build safely without a compiler to verify it against). Needs a manual test before merge: create a meeting with a recording → delete it → confirm both the DB rows and the `<title>_<timestamp>/` folder are gone.
- **Residual risk**: `meeting_notes` rows may still be orphaned on delete since `PRAGMA foreign_keys` is never enabled for the SQLite connection (per `security/reports/08-data-protection.md` §4) — out of scope for this fix, listed in `RESIDUAL_RISKS.md`. SQLite's default rollback/WAL journaling can still leave pre-delete bytes recoverable in the database file itself until a `VACUUM` — also out of scope, and not something application code can fully guarantee away.

---

## Items reviewed but explicitly not remediated this pass

- **Finding #3-10** from the Phase 1-9 summary (no strict offline mode, `open_external_url` command-injection primitive, unverified FFmpeg downloads, CI secret-prefix leak, `Cargo.lock` patch drift, committed `vs_buildtools.exe`, over-provisioned `fs:read-all`/`fs:write-all` capability, missing SBOM/CI scanning) — the repo owner asked to fix only #1 and #2 first; these remain open. See `RESIDUAL_RISKS.md`.
