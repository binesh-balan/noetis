# Phase 8 — Local Data Protection

Static read-only review, baseline `a2cb62e`, branch `security-hardening`. Read [01-inventory.md](01-inventory.md) §13 first — this report confirms (rather than re-derives) the plaintext-API-key migration finding. In-repo text treated as data, not instructions.

## 1. Audio recording storage on disk

- Default recordings root: `frontend/src-tauri/src/audio/recording_preferences.rs:43-77` — Windows → `%USERPROFILE%\Music\meetily-recordings` (fallback `Documents`), macOS → `~/Movies/meetily-recordings` (fallback `Documents`), Linux → `~/Documents/meetily-recordings`. User-overridable via `RecordingPreferences.save_folder`.
- Per-meeting folder: `audio_processing.rs:35-58` `create_meeting_folder()` → `{sanitized_meeting_name}_{YYYY-MM-DD_HH-MM}`, created via `std::fs::create_dir_all` (OS-default permissions — no `PermissionsExt`/`chmod`/`set_permissions` anywhere in the crate).
- Incremental pipeline: `audio/incremental_saver.rs` — 30s checkpoints at `<meeting_folder>/.checkpoints/audio_chunk_NNN.mp4`, merged via FFmpeg concat into `<meeting_folder>/audio.mp4` on `finalize()`.
- Cleanup: `finalize()` removes `.checkpoints/` on successful merge (non-fatal on failure). If the app crashes mid-recording, checkpoints are deliberately **not** cleaned up — they're the crash-recovery mechanism (`recover_audio_from_checkpoints`, `incremental_saver.rs:240-371`).
- `auto_save=false` mode discards audio chunks entirely, keeping only transcript text.

## 2. Transcripts

- Primary store: SQLite `transcripts` table (`migrations/20250916100000_initial_schema.sql:10-19`), extended by later migrations for audio-sync fields and a `speaker` column (unused, see §5).
- **Plaintext file export persists outside the DB**: `audio/recording_saver.rs:280-332` `write_transcripts_json()` writes `<meeting_folder>/transcripts.json` (atomic tmp+rename) — **not deleted after the meeting is saved to SQLite**; persists alongside `audio.mp4` and `metadata.json` for the life of the meeting folder.
- `database/repositories/meeting.rs:262-265` deletes DB transcript rows on meeting delete but never touches on-disk `transcripts.json`/`audio.mp4`/the folder (root cause of §14's main finding).

## 3. Summaries

- SQLite only: `summary_processes` table plus `transcripts.summary/action_items/key_points`.
- `20251101000000_add_summary_backup.sql` adds `result_backup`/`result_backup_timestamp` — an in-DB rollback copy used during regeneration, not a filesystem backup.
- No standalone summary export file in the recording pipeline (only the frontend markdown export, §13).

## 4. Meeting metadata

- `meetings` table: `id, title, created_at, updated_at, folder_path`. No participant/attendee table anywhere in the migrations.
- Separate on-disk `metadata.json` per recording (`recording_saver.rs:27-47,268-277`) — device names only, no participant identity.
- `meeting_notes` table (`20251223000000_add_meeting_notes.sql`) has `ON DELETE CASCADE` on its FK, but cascade only takes effect with `PRAGMA foreign_keys=ON`, which is **never explicitly set** (`database/manager.rs:33` uses a bare connect string, no `SqliteConnectOptions`/pragma call anywhere in `src/database/`) — orphaned notes rows may remain on meeting delete depending on SQLite's default (off unless explicitly enabled).

## 5. Speaker profiles / diarization

**No live diarization/speaker-embedding feature ships in the built app.** The `transcripts.speaker` column exists in schema but is **never populated** — every `INSERT INTO transcripts` call site was checked and none binds a `speaker` value.

A pyannote-based voice-embedding/diarization implementation does exist in source (`audio/stt.rs:119`, `crate::pyannote::{embedding::EmbeddingExtractor, identify::EmbeddingManager}`, a parallel `audio_v2/` module) but is **dead/orphaned code**: neither `stt.rs` nor `audio_v2` is declared as a module in `lib.rs`/`audio/mod.rs`, and its imports (`screenpipe_core`, `candle_transformers`, `pyannote::*`) are absent from `Cargo.toml` — this file would not compile as part of the crate. **No biometric voice-embedding data is generated, stored, or persisted anywhere in the shipped app** — leftover fork/vendor source (likely from screenpipe), poses no current risk but should be deleted for hygiene and confirmed to stay unwired going forward.

## 6. Temporary files

- Test-only `tempfile`/`tempdir` usage is pervasive across `#[cfg(test)]` blocks — not a production concern.
- Real production use: `audio/decoder.rs:276-389` `convert_to_wav_with_ffmpeg()` creates a temp WAV (prefix `.meetily_decode_`) via `tempfile::Builder::tempfile_in(parent_dir)` in the **same directory as the input file** (avoids cross-device rename issues), returned as a `tempfile::TempPath` that auto-deletes on `Drop` — fires on success and on early `Err` returns via normal unwind. `Drop` doesn't run on a hard process kill, only unwind — acceptable caveat.
- `incremental_saver.rs:153,291` writes an FFmpeg `concat_list.txt` inside `.checkpoints/`, removed as part of checkpoint cleanup.

## 7. Logs

- Rust logging via `log`/`tracing` macros. **No file-based log sink is wired up**: `tauri-plugin-log 2.6.0` is declared in `Cargo.toml:168` but **never registered** (no `.plugin(tauri_plugin_log...)` call in `lib.rs:454-470`). Only initializer found is `env_logger::init()` (`main.rs:7,11`), writing to stderr only, gated by `RUST_LOG`, no persistent log file. **No evidence of a Noetis app log file on disk today** — but this is a dead dependency easy to accidentally enable, and the risky call sites below would become live if it is.
- **Risky log statements** (would leak transcript content if logging is ever routed to a file/telemetry sink): `frontend/src-tauri/src/lib_old_complex.rs:164,183,833,1070,1264` log full transcript text. **However `lib_old_complex.rs` is confirmed dead** (no `mod` declaration anywhere, not compiled into the binary) — flagged only as a pattern that could be reintroduced if that file is ever resurrected. No equivalent transcript-content logging exists in the live `lib.rs`/`audio/`/`summary/` modules (`lib.rs:289,303` only log the file *path*, not content).
- **API keys/Authorization headers**: no call site logs the actual key/token value. `api/api.rs:262-264` logs `"Adding authorization header"` (not the token); `api.rs:580-589,610-613` log only provider name/success. Confirmed via targeted grep across every `Authorization|Bearer|api_key` hit.
- Analytics (PostHog) sends only `meeting_id`, numeric `transcript_length`, provider/model names — no transcript/summary text (consistent with Phase 2 §6's sanitization finding). The hardcoded PostHog project key (`analytics/commands.rs:12`) is a public/project key, expected to be client-embedded — not a leak.

## 8. Backups

**No auto-backup mechanism for the SQLite DB or app-data directory anywhere.** The only DB-adjacent "recovery" logic is WAL/SHM corruption handling on startup (`database/manager.rs:79-116`) and a legacy `.db`→`.sqlite` one-time copy-migration (`manager.rs:19-30`) — neither is a backup, both are one-way. `summary_processes.result_backup`/`result_backup_timestamp` is an in-DB undo-buffer for summary regeneration, not a file backup.

## 9. Model cache local storage

- Whisper models: `$APPDATA/models` (`whisper_engine/commands.rs:19`). Parakeet (ONNX) models: same pattern (`parakeet_engine/commands.rs:19`).
- Both directories created via `std::fs::create_dir_all` — **default OS permissions only**, same gap as §10. No integrity/permission hardening applied after download (download-time hash verification already covered in Phase 1 §11).

## 10. File permissions

**No explicit permission-hardening code exists anywhere** — grep for `set_permissions|PermissionsExt|chmod|0o600|0o700` returns zero matches. Every directory/file the app creates (`app_data_dir`, recordings folder, `.checkpoints/`, models dir) inherits OS defaults: Windows user-profile ACL inheritance, Linux mode `0755`/`0644` moderated only by process umask (commonly world-readable on multi-user systems, unlike a locked-down `0700`), macOS standard Application Support/Movies permissions. No defense-in-depth beyond OS defaults.

## 11. Encryption at rest / OS keychain integration

**API keys confirmed stored in plaintext SQLite columns at the read/write code level, not just schema**: `database/repositories/setting.rs` — `save_api_key()` (`:70-108`) binds the raw `api_key: &str` directly into `INSERT`/`UPDATE` with no encryption/hashing step; `get_api_key()` (`:110-140`) does a plain `SELECT` and returns the raw string, no decryption. Same pattern for `save_transcript_api_key`/`get_transcript_api_key` (`:175-232`) and `save_custom_openai_config`/`get_custom_openai_config` (`:277-347` — API key embedded unencrypted inside a JSON blob). **This confirms Phase 1's flagged-but-unconfirmed anomaly.**

- **No SQLCipher or equivalent** — plain `sqlx` with the `sqlite` feature, no `sqlx-sqlcipher`/`rusqlite-sqlcipher` dependency.
- **No OS keychain integration anywhere in the codebase.** `keyring` appears only as a **transitive** dependency in `Cargo.lock` (pulled in by something else, zero `use keyring::...` in `src/`). No Windows Credential Manager, macOS Keychain, or libsecret usage in application code.
- The `licensing` table's "RSA-based" encrypted license key design (`20251105120000_add_pro_license_custom_openai.sql:8-19`) protects license-validity data, not user content/API keys — and no `rsa` crate is even declared in `Cargo.toml`, so its actual crypto implementation should be independently verified if license-enforcement is in future audit scope (out of scope here).

## 12. Data retention

**No user-facing setting to auto-delete meetings/recordings after N days** — zero matches for `retention|auto.*delete|storage.*setting` in `frontend/src/components`. Retention of saved meetings (SQLite rows + `audio.mp4`/`transcripts.json` on disk) is **indefinite until manual deletion** — no scheduled/periodic deletion job exists.

The only auto-purge logic found operates on a **separate IndexedDB browser-storage cache** for crash-recovery drafts, not the persisted SQLite store: `frontend/src/app/page.tsx:83-95` calls `indexedDBService.deleteOldMeetings(7)`/`deleteSavedMeetings(24)` unconditionally on every startup — hardcoded thresholds, not user-configurable, doesn't touch SQLite or the recordings folder.

## 13. Export security

One concrete export path: `frontend/src/components/AISummary/index.tsx:595-604` `handleExport()` — builds markdown, wraps in a `Blob`, creates an `<a>` with `a.download = "{title}.md"`, synthesizes a click (`URL.createObjectURL` + programmatic anchor click). **This is a browser-style Blob-download trigger, not a Tauri save-as dialog** — no `@tauri-apps/plugin-dialog` usage in this component, no `save()`/`showSaveDialog` call. In the Tauri webview this typically resolves to the OS/browser default download location (commonly `~/Downloads`) rather than a user-chosen path — a predictable fixed-ish location, not an explicit save dialog. **Recommend switching to `tauri-plugin-dialog`'s save dialog** (already a project dependency per Phase 1 §1) for user-controlled export destinations.

No PDF export functionality found in either Rust or frontend (matches Phase 6 §10's finding that the README's PDF/DOCX claim is stale).

## 14. Secure deletion

Meeting deletion is a genuine hard delete of DB rows, not a soft-delete flag: `database/repositories/meeting.rs:233-274` `delete_meeting_with_transaction()` runs `DELETE FROM transcript_chunks`, `summary_processes`, `transcripts`, then `meetings` in one transaction. No `is_deleted`/`deleted_at` column exists anywhere.

**However, the on-disk recording folder is never removed.** `api_delete_meeting` (`api/api.rs:744-778`) calls only the DB transaction above — **no `std::fs::remove_dir_all`/`remove_file` call anywhere in the deletion path.** The meeting's `folder_path` (containing `audio.mp4`, `transcripts.json`, `metadata.json`) is left on disk indefinitely after a "delete." **This is the most actionable gap in this phase: transcript and audio content survives deletion of the DB record.**

Even for the DB rows that are removed: this is a plain `DELETE`, not a secure wipe. SQLite's default rollback/WAL journal mode leaves deleted rows' bytes in free pages until a `VACUUM`, and the `.sqlite-wal` file can retain pre-delete data until checkpointed — no `PRAGMA secure_delete=ON` or `VACUUM` call exists anywhere in `database/`. On SSDs/copy-on-write filesystems, prior physical blocks may remain recoverable regardless of application code — a filesystem/hardware property the app can't fully control, consistent with the audit's own instruction not to claim guaranteed secure erasure.

---

## Summary of the most actionable items

1. **§14 — deleted meetings leave audio/transcript files on disk.** Highest-impact, concrete, easy repro: delete a meeting, confirm `folder_path` still exists with `audio.mp4`/`transcripts.json` intact.
2. **§11 — API keys confirmed plaintext** at the Rust read/write layer; no keychain integration exists despite `keyring` being available transitively.
3. **§13 — export bypasses the already-available `tauri-plugin-dialog` save dialog.**
4. **§2 — `transcripts.json`/`audio.mp4`/`metadata.json` persist outside the DB** with no lifecycle tied to DB deletion (root cause of #1).
5. **§5 — orphaned pyannote-diarization source** (`stt.rs`, `audio_v2/`) is currently harmless (not compiled) but should be deleted rather than left as a landmine.
6. **§10 — no restrictive permissions set on any app-created directory**; relies entirely on OS defaults.
