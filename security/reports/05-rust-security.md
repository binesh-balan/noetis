# Phase 5 — Rust Security Review

Static read-only review, baseline `a2cb62e`, branch `security-hardening`. No `cargo build`/`cargo run` executed. Builds on [01-inventory.md](01-inventory.md) and [02-network-audit.md](02-network-audit.md) — not re-derived here.

**Scope note on dead code**: `frontend/src-tauri/src/lib_old_complex.rs`, `src/audio/recording_saver_old.rs`, `src/audio/core-old.rs`, and the entire `src/audio_v2/` module are **not referenced by any `mod` declaration** and confirmed not compiled. They contain the bulk of raw `unsafe`/`static mut` usage in the tree, but none of it ships. Excluded from findings below except where noted; flagged once here as cleanup candidates.

## 1. `unsafe` blocks (compiled code only)

| # | File:Line | What it does | Documented? |
|---|---|---|---|
|1|`audio/capture/core_audio.rs:193-195`|Fallback manual `slice::from_raw_parts` when the safe buffer path fails; guarded by null/count check just above|Adequate, no explicit SAFETY comment|
|2|`audio/device_monitor.rs:98-102,133,157`|`unsafe impl Send`; raw pointer deref in Core Audio extern callback; `Box::into_raw`/`from_raw` for a leaked callback context|**Well documented** — `:113-119` explains the deliberate leak avoids a use-after-free since `AudioObjectRemovePropertyListener` doesn't fence in-flight callbacks|
|3|`audio/pipeline.rs:64-71`|`static mut SAMPLE_COUNTER: u64`, incremented per `add_samples` call for a debug log|**Not documented, technically unsound** — process-global static mutated without synchronization is UB if ever hit from more than one `AudioPipeline` instance concurrently. Low practical impact (worst case: torn counter), but a real unsoundness pattern. Trivial fix: `AtomicU64`.|
|4|`audio/recording_manager.rs:194-197`|`unsafe impl Send for RecordingManager {}`|**Weak** — comment is circular ("contains types we've marked Send"), doesn't restate why the inner `cpal` types are safe to move|
|5|`audio/stream.rs:29,38,363`|`unsafe impl Send` for `StreamBackend`/`AudioStream`/`AudioStreamManager` (wrapping `cpal::Stream`, which is `!Send`)|Documented but assertion-based, not compiler-enforced — relies on `spawn_blocking` thread-confinement discipline, not something the type system checks|
|6|`audio/system_detector.rs:150,160,205,217`|macOS-only Core Audio property-listener callbacks, raw pointer deref/slice|Same pattern/rigor as row 2|
|7|`console_utils/console_utils.rs:26,71,112`|Windows `kernel32` FFI (`GetConsoleWindow`/`ShowWindow`/`AllocConsole`)|Standard, low-risk, null-checked. Fine.|

The only genuinely concerning unsafe usage in compiled code is `static mut SAMPLE_COUNTER` (row 3) — everything else is macOS Core Audio C-callback marshalling with a correct leak-vs-UAF tradeoff, or WinAPI FFI with null checks.

## 2. FFI boundaries — whisper-rs, ort/ONNX, llama-cpp-2

- **whisper-rs**: used only through its safe wrapper API — zero raw `unsafe` blocks touch whisper-rs objects.
- **ort/ONNX Runtime**: safe `ort` 2.0-rc API exclusively (`Session::builder()`, `TensorRef`, `ndarray` views) — no raw pointer/FFI code, ownership crosses the boundary through owned values.
- **llama-cpp-2**: safe wrapper calls only — **zero `unsafe`** in `llama-helper` (confirmed via grep).

For all three named FFI surfaces, app-level Rust code contains no manual pointer/lifetime handling — the actual unsafe internals live inside the third-party crates, out of this static review's reach without building. The only hand-rolled FFI/callback pointer work is the macOS Core Audio integration (§1 rows 1,2,6) via `cidre`.

**Minor arithmetic note**: `llama-helper/src/main.rs:389` — `(tokens_list.len() - 1) as i32` would underflow if `str_to_token` ever returned an empty vec; `AddBos::Always` (`:380`) guarantees at least one token today, so unreachable, but unchecked.

## 3. Unchecked integer arithmetic

Swept `whisper_engine.rs`, `parakeet_engine.rs`, `llama-helper/src/main.rs`, `audio/decoder.rs`, `audio/stt.rs`, `audio/post_processor.rs`. One theoretical underflow (§2, currently unreachable); `audio/post_processor.rs:175`'s `words.len() - 1` is protected by short-circuit evaluation (fragile if reordered, but safe as written). No other unchecked-cast-into-buffer-size patterns found — download-size-verification code already validates expected byte counts before allocation. **Clean.**

## 4. Unchecked indexing on external/untrusted input

- Parakeet vocab decode (`parakeet_engine/model.rs:439-450`) explicitly bounds-checks `idx < self.vocab.len()` before indexing, even though the ID nominally comes from a hash-verified model.
- Vocab loading (`model.rs:145-181`) computes `max_id` as a running max across the file *before* allocating the vector, so `id <= max_id` always holds regardless of file content — worst case is a large allocation (DoS/memory-exhaustion vector), not memory corruption.
- `whisper_engine.rs` word-repetition post-processing (`:469-530`) — all indexing guarded by `while i < words.len()`.
- No indexing driven by network-response or file-header length fields without a prior bounds check.

## 5. Panic handling on untrusted-input paths

- `whisper_engine.rs` production code: one `.expect()` (`:206`), an internal mutex-poison invariant, not attacker-reachable.
- `parakeet_engine.rs` production code: one `.expect()` (`:1143`), internal download-progress invariant, not attacker-reachable.
- `summary/llm_client.rs` production code: **zero** `.unwrap()`/`.expect()` — all hits are in the `#[cfg(test)]` mock-server harness.
- `audio/decoder.rs` (parses user-imported audio files via `symphonia` — the one genuine "parse untrusted file" path): **zero** `.unwrap()`/`.expect()` in production code.
- Cloud-provider response parsing (`anthropic.rs`, `openai.rs`, `groq.rs`, `ollama.rs`, `openrouter.rs`): routes through `?`/serde deserialization, no manual JSON indexing with `.unwrap()`.
- `retranscription.rs:373,380`/`import.rs:584,591` — `.as_ref().unwrap()` calls verified sound: gated by the same boolean that decided which `Option` gets populated earlier in the same function.
- `recording_state.rs`, `recording_commands.rs`, `analytics/commands.rs`: dozens of `Mutex::lock().unwrap()`. Standard idiom (panics only on lock poisoning), but a **panic-cascade risk**: if any critical section under `RECORDING_MANAGER`/`ANALYTICS_CLIENT` panics, every subsequent lock call anywhere panics too — one localized bug could become an app-wide crash loop. Worth a defensive follow-up (`parking_lot` or `.unwrap_or_else(|e| e.into_inner())`), not urgent.

**Overall**: panic-on-untrusted-input is essentially absent from the audited hot paths — the places parsing untrusted bytes deliberately propagate `Result` rather than panicking.

## 6. File-system path construction from user input

`sanitize_filename()` (`audio/audio_processing.rs:15-25`) strips `/ \ : * ? " < > |` and control characters from meeting titles before they become folder-name components. `create_meeting_folder()` (`:35-58`) always appends a `_YYYY-MM-DD_HH-MM` timestamp suffix after sanitizing, so even a title of literal `".."` can never become a bare `".."` path component — **path traversal via meeting title is not exploitable**. Consistently reused (`import.rs:344`, `recording_saver.rs:229`); no code path bypasses it.

Tauri IPC commands accepting a raw path string without server-side allowlisting (`database/commands.rs:48,109`, `audio/import.rs:958`) do existence/metadata checks only, no allowlist — but this isn't a new privilege boundary given the app's already-documented `fs:read-all`/`fs:write-all` capability grant (Phase 1 §4); a compromised webview already has equivalent access via the raw `fs` plugin. These commands just add no defense-in-depth on top of that broad grant.

**No exploitable path-traversal found.**

## 7. Symlink handling

No symlink checks anywhere in runtime code (`src/`) — zero hits for `symlink`/`is_symlink`/`read_link`. The only symlink-aware code in the crate is the **build script** (`build/onnxruntime.rs:71,240,254,268`, already noted positively in Phase 1 §5, checked at build time). At runtime the app operates on its own app-data dir, user-picked paths via native dialogs, and self-created meeting folders — none read/written through a check-then-act-on-different-path pattern a symlink swap could exploit in the reviewed code, but there's also no explicit `O_NOFOLLOW`-equivalent hardening. Low real-world risk for a single-user desktop app; worth noting if a recordings folder ever became attacker-writable (e.g. a shared/synced folder scenario).

## 8. Temp file creation

All production temp-file usage goes through the `tempfile` crate (`audio/decoder.rs:279-296`, `tempfile::Builder::new().tempfile_in(parent_dir)`), which uses random non-predictable names and exclusive creation — no hand-rolled `format!("/tmp/foo-{}", predictable_id)` pattern found anywhere. Non-tempfile "temp" files (`.metadata.json.tmp`/`.transcripts.json.tmp` atomic-write staging files) live **inside the meeting's own folder**, not a shared/world-writable temp dir, then get `fs::rename`d over the final file — not a shared-tmp-dir race. **No insecure temp-file pattern found.**

## 9. Race conditions

- `static mut SAMPLE_COUNTER` (§1 row 3) — the only genuine unsynchronized-shared-state finding in compiled code; low practical impact.
- Everything else needing cross-task shared mutable state is properly `Mutex`/`RwLock`-protected (`RECORDING_MANAGER`, `TRANSCRIPTION_TASK`, `ANALYTICS_CLIENT`, `whisper_engine.rs:70`'s `Arc<RwLock<...>>`).
- **TOCTOU on files**: the atomic write-to-`.tmp`-then-`rename` pattern avoids check-then-use races on writes. On the read side (`database/commands.rs:54,64-66,73-74`), a `path.exists()` check precedes a later open with a theoretical TOCTOU window — but since it operates on a path the same local user just picked via a native file dialog, practical exploitability (requires racing your own filesystem at the same privilege level) is minimal.
- No lock-ordering/deadlock-prone patterns spotted.

## 10. Deserialization of untrusted input

`serde_json` throughout (cloud-LLM API responses, Tauri command payloads, DB-stored JSON config, `llama-helper` stdio IPC) — all **derive-based `#[derive(Deserialize)]`**, no custom `Deserialize` impls with manual unsafe assumptions found anywhere (grepped, zero hand-written impls). `llama-helper/src/main.rs:589` deserializes stdin input (from the parent Tauri process, not network-exposed) with proper `Err` handling, no panic on malformed input. Cloud-provider responses handled via `?`/`match` error propagation. No `bincode` usage anywhere. **Clean.**

## 11. Command injection

Every `Command::new(...)` call site was inventoried; nearly all are argv-based with fixed programs/args (ffmpeg via bundled sidecar path, `osascript` with a compile-time literal script, `nice`/helper-binary launches with user data going over stdin JSON, not argv). One real finding:

**`frontend/src-tauri/src/api/api.rs:1148-1165`, `open_external_url`:**
```rust
#[tauri::command]
pub async fn open_external_url(url: String) -> Result<(), String> {
    let result = if cfg!(target_os = "windows") {
        Command::new("cmd").args(&["/C", "start", &url])
    } ...
```
A `#[tauri::command]`, invocable by any webview JS via `invoke('open_external_url', {url})` with `url` fully attacker-controlled from the IPC caller's perspective. No scheme validation, no metacharacter sanitization before `cmd /C start <url>` on Windows — `cmd.exe`'s `/C` mode re-tokenizes for shell metacharacters (`&`, `\|`, `^`, `%`) independent of the original argv split, so a value like `http://x & calc.exe` is a classic Windows command-injection vector if an attacker ever gets a string into this call. All current call sites are hardcoded literals (`About.tsx:26`, `AnalyticsConsentSwitch.tsx:150`, `ModelSettingsModal.tsx:739,1236`) — **not exploitable through the shipped UI today** — but it's an unguarded primitive sitting on the IPC boundary; any future feature turning transcript/imported content into a clickable link, or any webview XSS, would have a ready-made gadget. (Matches Phase 4 §4's independent finding of the same issue.) **Recommend validating `url` starts with `http://`/`https://`, or using `tauri-plugin-opener`'s scheme-restricted `open_url`.**

## 12. SQL injection

The large majority of queries use `sqlx::query!`/`query_as!`/parameterized binds (44 call sites) — safe. **Raw string-built SQL via `format!`** exists in `database/repositories/setting.rs` (`save_api_key`, `get_api_key`, `save_transcript_api_key`, `get_transcript_api_key`, `delete_api_key`) — interpolates a *column name* into SQL text, the textbook injection-risk pattern. However, the interpolated value is always one of a small hardcoded set of literals selected by a `match` with an explicit `_ => Err(...)` fallthrough — the attacker-controlled `provider` string never reaches the query text itself, only selects which literal is used or gets rejected. **Not exploitable as written.** Fragile pattern though — relies on every future `match` arm continuing to map to a hardcoded literal; a future edit doing `provider.to_string()` as a fallback instead of erroring would turn this into a real injection. **Recommend a compile-time-checked enum** instead of `match`-on-`&str` + `format!`, or at minimum a code comment flagging the invariant.

## 13. IPC authentication (Tauri commands)

171 `#[tauri::command]` handlers total. No app-level authentication layer exists beyond `tauri.conf.json`'s static capability grants — expected for a single-user local desktop app where webview and Rust backend share a trust domain by design.

**Concrete finding**: seven command handlers in `api/api.rs` (lines 470, 525, 578, 603, 656, 691, 721) accept an `_auth_token: Option<String>` parameter that is **never read or validated** (underscore-prefixed = intentionally unused). The one helper that could use it, `get_auth_token()` (`api.rs:206-208`), is `#[allow(dead_code)]` and isn't called from any of these seven. Vestigial plumbing from an earlier or aspirational auth design that was never wired up. One example, `api_get_api_key` (`api/api.rs:573-597`, registered `lib.rs:722`):
```rust
pub async fn api_get_api_key<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    provider: String,
    _auth_token: Option<String>,   // accepted, never checked
) -> Result<String, String> {
    match SettingsRepository::get_api_key(&state.db_manager.pool(), &provider).await {
        Ok(key) => Ok(key.unwrap_or_default()),
```
Any JS in the webview can call `invoke('api_get_api_key', {provider: 'openai'})` and get the **plaintext API key back** (§17) with zero gating beyond the process boundary itself. Realistic threat model for a single-tenant local app: webview compromise (XSS via rendered meeting content, a malicious imported file, or a compromised npm dependency) → full read access to every stored provider API key via one IPC call, despite a parameter existing for exactly that purpose and never being wired up. **Recommend either wiring `_auth_token` to a value only the legitimate frontend session holds, or removing the dead parameter and documenting "webview = Rust backend" as the accepted trust boundary.**

## 14. WebSocket origin validation

Confirms and extends Phase 3: **no WebSocket/TCP server exists in the compiled Rust/Tauri codebase.** Every `TcpListener`/`axum`/`warp`/`hyper::Server` hit outside test harnesses is either dead code (`lib_old_complex.rs:72`) or a client. The actual `127.0.0.1:8178` server is a **C++** component of the legacy/archived Python backend (`backend/whisper-custom/server/server.cpp`), started with `--host 127.0.0.1` in Windows scripts, out of Rust-review scope. **The shipped Tauri app has no origin-validating (or non-validating) WebSocket server of its own** — the frontend's reference to `127.0.0.1:8178/stream` targets a process the current Rust app doesn't start.

## 15. CORS configuration

- Rust side: no HTTP server exists in the Tauri app — N/A.
- Python FastAPI backend (`backend/app/main.py:44-48`), reviewed despite being outside Rust-only scope:
  ```python
  app.add_middleware(CORSMiddleware, allow_origins=["*"],  # Allow all origins for testing
  ```
  Wide open. Legacy/archived component (Phase 1 §3, Phase 2 §7, Phase 3) — not part of the default Tauri bundle, but a real gap if a user runs it standalone, compounded by the `0.0.0.0` bind already flagged in Phase 2/3.

## 16. SSRF

Already covered in depth by Phase 3: `is_localhost_endpoint()` (`ollama.rs:68-78`) exists but is not used to block anything — only affects CLI fallback behavior on HTTP error. Nothing validates or restricts the custom OpenAI endpoint (`llm_client.rs:323-330`) or Ollama endpoint before making outbound requests — a user (or a malicious imported settings/config file, if one exists) setting either endpoint to an internal address causes the app to request it with no server-side check. Confirmed the Rust-side code matches what Phase 3 documented; see that report for full analysis and remediation.

## 17. Secrets handling (API key storage)

**Definitive evidence from the actual read/write code: plaintext at rest, no encryption, no OS keychain integration anywhere in the Rust tree.**

- Storage: `database/repositories/setting.rs:105` (`save_api_key`) and `:203` (`save_transcript_api_key`) bind the raw API-key string directly into a `TEXT` SQLite column, no transformation.
- Schema: `migrations/20250916100000_initial_schema.sql:56-59,67-71` declares `groqApiKey TEXT`, `openaiApiKey TEXT`, `anthropicApiKey TEXT`, `ollamaApiKey TEXT`, `whisperApiKey TEXT`, `deepgramApiKey TEXT`, `elevenLabsApiKey TEXT` — plain `TEXT`, no encrypted-column convention. Later migrations follow the same pattern.
- Retrieval: `setting.rs:138` returns the key as a plain `String` — including, per §13, directly to any webview JS calling `api_get_api_key`.
- **No encryption/keychain integration exists**: grepped the entire `frontend/src-tauri/src` tree for `keychain`, `credential_manager`, `dpapi`, `libsecret`, `keyring`, `encrypt` (case-insensitive) — **zero matches**. No Windows Credential Manager, no macOS Keychain, no libsecret, no SQLCipher/at-rest DB encryption. The SQLite file itself is unencrypted on disk.

This confirms and closes Phase 1 §13's open item with definitive evidence: **keys are not encrypted at rest.** Combined with §13's finding that retrieval has no working auth gate, this is a **two-layer gap** — storage: plaintext; retrieval: ungated. **Recommend OS keychain integration** (e.g. the `keyring` crate, which supports Windows Credential Manager/macOS Keychain/libsecret uniformly) as the standard fix.

## Dependency advisories (cargo-audit)

`cargo`/`cargo-audit` are **not installed** in this environment — not attempted per instructions. **Recommend installing `cargo-audit`** and running it against `Cargo.lock` for both workspace members as a follow-up, particularly given Phase 1's `cpal`/`esaxx-rs` lockfile-drift finding — exactly the kind of thing `cargo audit`/`cargo tree` would help clarify.

---

## Summary of concrete, actionable findings (severity-ordered)

1. **Plaintext API keys, ungated retrieval** (§13, §17) — `api_get_api_key` (`api/api.rs:573-597`) returns plaintext provider API keys to any webview JS with no working auth check (the `_auth_token` parameter is dead), and keys are stored unencrypted in SQLite. **Highest-impact finding of this phase.**
2. **`open_external_url` cmd.exe injection primitive** (§11) — unvalidated `url: String` from any IPC caller into `cmd /C start <url>` on Windows. Not exploitable via today's UI, but an unguarded gadget on the IPC boundary.
3. **`static mut SAMPLE_COUNTER`** (§1, §9) — unsynchronized global mutable state; low practical impact, trivial fix.
4. **Fragile-but-currently-safe `format!`-built SQL column names** (§12) — safe today only because the interpolated value always comes from a hardcoded match arm; recommend hardening against regression.
5. **`unsafe impl Send` justified only by runtime discipline** (§1) — standard pattern for `cpal`-based audio code, worth a stronger safety comment.
6. Dead code (`lib_old_complex.rs`, `recording_saver_old.rs`, `core-old.rs`, `audio_v2/`) contains most raw unsafe/static-mut patterns but doesn't compile — safe to delete, shrinks unsafe surface for future reviewers.
7. `cargo-audit` not installed — run it as a follow-up, and re-check the Phase 1 `cpal`/`esaxx-rs` lockfile-drift anomaly while there.

**No findings for**: FFI memory-ownership issues in whisper-rs/ort/llama-cpp-2 (all safe-wrapper usage), unchecked-indexing-from-untrusted-input beyond what's already bounds-checked, insecure temp-file creation, custom-`Deserialize`-impl risk (none exist), exploitable path traversal via meeting titles.
