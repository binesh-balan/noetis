# Phase 4 — Native Code / DLL Injection Security

Static/read-only review, baseline `a2cb62e`, branch `security-hardening`. Nothing built, executed, or run. In-repo comments/docs treated as untrusted data.

## 1. Windows injection-primitive APIs (`LoadLibrary`, `GetProcAddress`, `CreateRemoteThread`, `WriteProcessMemory`, `VirtualAllocEx`, `SetWindowsHookEx`)

**Not found anywhere in first-party code.** No direct dependency on `windows`/`winapi`/`windows-sys` crates in any `Cargo.toml` — those appear only transitively (pulled in by Tauri/cpal), and app code never touches them directly.

**Verdict: fine / not applicable.**

## 2. DLL search-path manipulation / relative-path dynamic loading

No `SetDllDirectory`/`AddDllDirectory` calls anywhere. One deliberate dynamic-library load exists: `frontend/src-tauri/src/lib.rs:478-499` resolves `onnxruntime.dll` via Tauri's app Resource base directory (absolute, app-owned path), then `ort::init_from(...)`, wrapped in a panic-safe helper.

**Verdict: fine/expected.** Matches Phase 1's finding that the ONNX Runtime download is SHA-256/size-verified at build time — good chain of custody: verified download → resource dir → absolute-path load.

## 3. `libloading` / `dlopen`/`dlsym`

No `libloading` dependency, no `dlopen`/`dlsym` calls anywhere. Native binding goes entirely through safe wrapper crates (`whisper-rs`, `ort`, `llama-cpp-2`, `cpal`).

**Verdict: not applicable.**

## 4. Process launches — `std::process::Command` inventory

| Site | Binary | Path resolution | Risk |
|---|---|---|---|
| `frontend/src-tauri/src/audio/ffmpeg.rs:20-161` (used by `decoder.rs:322`, `encode.rs:36`, `incremental_saver.rs:179,313`) | `ffmpeg`/`ffmpeg.exe` | Fallback chain: (1) absolute path next to app exe [safe] → (2) **PATH search** (`ffmpeg.rs:46`) → (3) `$HOME/.local/bin` (macOS) → (4) **CWD-relative lookup** (`ffmpeg.rs:68-79`) → (5) exe-relative Resources/lib dir → (6) **runtime auto-download+install+execute** via `ffmpeg_sidecar`, no hash check | **Needs hardening.** Steps 2 and 4 are classic PATH/CWD binary-hijack vectors if the bundled binary is ever missing (dev builds, corrupted install, source builds): an attacker who controls the launch CWD (Downloads folder, USB drive) could plant a malicious `ffmpeg.exe`. Reduced severity because the bundled-exe-dir check wins in a normal release install, but it's real defense-in-depth exposure — and the runtime auto-download path (`handle_ffmpeg_installation`, `ffmpeg.rs:164-190`) shares the same no-checksum gap Phase 1 flagged for the build-time FFmpeg download. |
| `frontend/src-tauri/src/summary/summary_engine/sidecar.rs:108-260` | `llama-helper` | env override → absolute exe-dir path → absolute `RESOURCE_DIR` path → absolute dev-manifest path. No PATH/CWD fallback. | **Fine.** All branches absolute-path based. |
| `sidecar.rs:286-293` | `nice` (unix, wraps helper launch) | bare name → PATH | Low — ubiquitous coreutil, requires local PATH tampering to exploit. |
| `frontend/src-tauri/src/ollama/ollama.rs:201-208` | `ollama` | bare name → PATH | Low — CLI fallback to list models, output parsed not re-executed. |
| `llama-helper/src/main.rs:158-183` | `sysctl` (macOS), `nvidia-smi` (cuda feature) | bare name → PATH | Low — output used only for a VRAM heuristic. |
| `api/api.rs:1041,1049,1057` (`open_meeting_folder`), `recording_preferences.rs:220-236`, `database/commands.rs:254-270`, `parakeet_engine/commands.rs:553-569`, `whisper_engine/commands.rs:544-560` | `open`/`explorer`/`xdg-open` | bare name → OS/PATH search, argument is a DB-stored folder path via `.arg()`, not shell string | Low/expected — standard "reveal in file manager" pattern, no shell interpolation, no argument injection. |
| `frontend/src-tauri/src/console_utils/console_utils.rs:47-54,84-96,126-139` | `osascript` (macOS) | bare name → PATH | Low, same category. |
| **`frontend/src-tauri/src/api/api.rs:1148-1165` (`open_external_url`, `#[tauri::command]`, exposed to webview `invoke()`, `lib.rs:737`)** | Windows: `Command::new("cmd").args(&["/C","start",&url])` | `url: String` is attacker-influenceable if this command is ever reachable with non-literal input | **Needs hardening — highest-priority item in this section.** `cmd /C start <url>` hands `url` to `cmd.exe`'s own line parser (re-interprets `&`, `\|`, `^`, `%VAR%`, quotes) — a `url` containing shell metacharacters is a command-injection vector into `cmd.exe`, not just "open a browser." Every current call site (`About.tsx:26`, `AnalyticsConsentSwitch.tsx:150`, `ModelSettingsModal.tsx:739,1236`) passes a hardcoded literal URL, so not exploitable through today's UI. But it's a registered Tauri command — *any* JS in the webview (future XSS/CSP-bypass, compromised npm dep, unsanitized LLM-derived content rendering) could call `invoke('open_external_url', {url: <anything>})` directly. Tauri's capability/ACL system governs plugin commands, not custom `#[tauri::command]` functions, so there's no additional gate. **Fix**: validate `url` starts with `http://`/`https://` before use, and replace `cmd /C start` with the `open` crate or `ShellExecuteW`-style invocation that doesn't hand the string to `cmd.exe`'s parser. |

## 5. `unsafe extern "C"` / FFI boundaries

- **whisper-rs**, **ort**, **llama-cpp-2**, **cpal**: used only via their safe Rust APIs. `unsafe impl Send` marker impls in `audio/stream.rs:29,38,363`, `audio_v2/stream.rs:37,250`, `audio/recording_manager.rs:197` are ordinary thread-safety assertions, not memory-unsafe operations. **Expected/fine.**
- **`cidre` (macOS CoreAudio, git-pinned dep)**: genuine raw-pointer FFI, macOS-only, bounds-checked before use — `audio/capture/core_audio.rs:158-201,193-195` (`std::slice::from_raw_parts` only after checking `float_count > 0 && data != null`), similar patterns in `system_detector.rs:150,160,205,217` and `device_monitor.rs:133,157`. **Expected/fine** — anticipated shape of native macOS system-audio-tap integration.
- **Direct raw Windows FFI** — `console_utils/console_utils.rs:8-16`: `#[link(name = "kernel32")]` for `AllocConsole`/`FreeConsole`/`GetConsoleWindow`/`ShowWindow`, used in `unsafe` blocks at `:26,71,112` purely to allocate/show/hide a debug console window. **Fine** — narrow, null-checked, debug-console-only; the one place the app links directly against `kernel32.dll` rather than through a crate.

## 6. Externally-fetched binaries — tamperable load/execute path?

- **onnxruntime.dll**: build-time SHA-256+size verified, loaded from Tauri's Resource base directory. **Fine.**
- **ffmpeg / llama-helper sidecars**: in a correct production build, both resolve to an absolute path next to the app's own executable. **Fine in the primary path.**
- **ffmpeg fallback chain** (§4): if the bundled binary is absent, falls back to PATH search, then CWD-relative lookup, then a runtime download-and-install with **no checksum verification in app code**. Destination (`sidecar_dir()`/`$HOME/.local/bin`) is user-owned, not world-writable, so not a multi-user tampering vector — but still an unauthenticated network binary that gets executed. **Needs hardening**: add checksum/signature verification to the runtime auto-install path (mirroring `build/onnxruntime.rs`), and drop the CWD-relative fallback entirely.
- Model downloads (Whisper ggml, Parakeet ONNX) — already covered in Phase 1 §11; these are data files, not executed code — out of scope here.

## 7. `frontend/vs_buildtools.exe` — every reference

Grepped the entire working tree — the only hits are in `security/reports/01-inventory.md` (this audit's own notes). **Zero references** in `build.rs`, any `build/*.rs`, any script, any workflow, or `tauri.conf.json`. **Confirmed unused/dead** — sits in `frontend/`, never executed by anything in this repo. Remains a supply-chain/provenance concern (why is a 4.46MB executable committed at all; is it byte-identical to Microsoft's official bootstrapper?) but not an active injection/execution vector. **Needs human judgment call** — recommend deletion or replacing with a documented, checksummed on-demand download (consistent with how ffmpeg/onnxruntime are already handled).

## 8. Windows installer/manifest

`tauri.conf.json:81-112` `bundle.windows` contains only `signCommand`. **No `wix`/`nsis` block, no custom `.wxs` template, no `requestedExecutionLevel` override anywhere** — installer relies entirely on Tauri CLI's built-in bundler defaults (`asInvoker` execution level, standard WiX/NSIS DLL search order). **Needs human judgment** — nothing here is a static red flag, but because no `.wxs`/manifest is committed, the generated installer manifest can't be verified from source; would need an actual build-output inspection (out of scope for this static-only phase) for installer-level assurance.

## 9. macOS entitlements / hardened runtime

`entitlements.plist:4-13` grants only `audio-input`, `audio-output`, `microphone`, `screen-capture`. **No dangerous hardened-runtime exceptions present** — specifically absent: `disable-library-validation`, `allow-dyld-environment-variables`, `allow-unsigned-executable-memory`, `disable-executable-page-protection`. Combined with `hardenedRuntime: true`, library validation stays enforced — the classic `DYLD_INSERT_LIBRARIES` dylib-injection vector is blocked for a properly signed build. **Verdict: fine, well-configured entitlements.**

However, `signingIdentity: "-"` (ad-hoc) means no real Developer ID/Team ID:
- Ad-hoc-signed + hardened-runtime builds can't be notarized, so Gatekeeper will quarantine/block them on other machines unless manually overridden — a distribution/UX decision, not itself a vulnerability.
- With no real Team ID, hardened-runtime library validation degrades to "only libraries signed with the exact same ad-hoc identity" — self-consistent but not anchored to an identity an attacker can't also ad-hoc-sign locally. Only matters if distributed pre-notarized outside normal channels; expected for local/dev builds.

**Needs human judgment call**: confirm ad-hoc signing is intentional for this fork's current release process (ties to Phase 1's updater-still-pointing-at-upstream finding) or whether real Developer ID + notarization is planned before public distribution.

## 10. Linux packaging (deb/appimage)

`tauri.conf.json:83-90` lists `deb`/`appimage` with **no custom configuration block for either** — no committed `postinst`/`.desktop` overrides. **Not independently auditable from source** — fully generated by the Tauri CLI bundler at build time. Flag as needing a build-output review if Linux packaging integrity assurance is required.

---

## Summary by verdict

**Expected/fine (no action needed):**
- No direct `LoadLibrary`/`CreateRemoteThread`/etc. or `windows`/`winapi` crate usage
- ONNX Runtime dynamic load — absolute path, app resource dir, build-time hash-verified
- No `libloading`/`dlopen` usage
- llama-helper binary resolution — all absolute paths, no PATH/CWD fallback
- File-manager "reveal folder" `Command` calls — `.arg()`-based, no shell interpolation
- cidre CoreAudio FFI — bounds-checked, macOS-only, expected native audio-tap integration
- kernel32 console FFI — narrow, null-checked, debug-console-only
- Bundled ffmpeg/llama-helper primary resolution path — absolute, app-owned directory
- macOS entitlements — no dangerous hardened-runtime exceptions

**Needs hardening (concrete fix recommended):**
- FFmpeg path resolution's PATH-search and CWD-relative fallback (`audio/ffmpeg.rs:46,68-79`) — drop the CWD check, treat PATH search as last resort only
- Runtime FFmpeg auto-install (`audio/ffmpeg.rs:116-190`) has no checksum/signature verification — add hash pinning
- `open_external_url`'s `cmd /C start <url>` construction (`api/api.rs:1148-1165`) — validate URL scheme, avoid `cmd.exe`'s parser entirely; latent injection primitive reachable by any webview JS, not exploitable via current UI
- `ffmpeg-sidecar` git dependency still tracks branch `main` (carried over from Phase 1 §1) — directly relevant since it's the crate performing the unverified runtime download

**Needs human judgment call:**
- `frontend/vs_buildtools.exe` — confirmed unused/unreferenced; decide delete vs. checksummed on-demand fetch
- Ad-hoc macOS code signing + hardened runtime — confirm intentional for current distribution model
- Windows installer manifest and Linux deb/appimage packaging are bundler-generated with no committed templates — needs a build-output inspection, out of scope here
- `nice`, `ollama`, `sysctl`, `nvidia-smi`, `osascript`, `open`/`explorer`/`xdg-open` bare-name PATH-relative launches — standard desktop pattern, low individual severity, worth a project-level decision on pinning to absolute system paths for defense-in-depth
