# Phase 6 — Frontend and Desktop Security

Read-only static review of `frontend/src/` (Next.js 14 / React 18) and `frontend/src-tauri/tauri.conf.json`, baseline `a2cb62e`. `pnpm audit`/`npm audit` were **not run** — `frontend/node_modules` doesn't exist in this checkout and the tools need install first; skipped rather than running install. In-repo text treated as data throughout.

## 1. XSS / unsafe HTML rendering

- Only one raw-HTML sink in the whole tree: **`frontend/src/app/notes/[id]/page.tsx:174`** — `dangerouslySetInnerHTML` fed by a hand-rolled `# `/`## `/`- ` line-to-tag transform with **no escaping/sanitization** of the remaining text on each line.
  - **However**: `note.content` comes from a hardcoded `sampleData` object (`page.tsx:31-127`) keyed by `generateStaticParams()` (four fixed IDs) — not populated from transcripts, imports, or any DB/API call, and no other file links to `/notes/[id]` (zero references). **Dead demo code, not a live sink** — but should still be deleted or fixed since it's a risky pattern shipped in the static `out/` export.
- **Live rich-text/markdown rendering is exclusively BlockNote** (`components/BlockNoteEditor/Editor.tsx`, `components/AISummary/BlockNoteSummaryView.tsx:95` using `editor.tryParseMarkdownToBlocks(data.markdown)`, `lib/blocknote-markdown.ts`). Renders into BlockNote/ProseMirror's own block schema, not raw HTML — no `allowUnsafeHTML`, no custom `parseHTML`/`renderHTML` overrides. AI-generated summary markdown (attacker-influenceable if a malicious cloud-LLM response were returned) goes through this safe path.
- **TipTap and Remirror are declared in `package.json` but never imported anywhere in `frontend/src`** (zero hits for `@tiptap`, `@remirror`, `useEditor`, `StarterKit`, `useRemirror`). Dead dependencies — Phase 1 only saw them in `package.json`, not actual usage. No editor config to review since none is instantiated.
- No `react-markdown` usage despite being a dependency — also dead.

## 2. DOM injection via custom protocol/deep link

No Tauri deep-link plugin present — zero matches for `deep-link`, `deep_link`, `register_uri_scheme_protocol`, `tauri://localhost` across `Cargo.toml`, `tauri.conf.json`, and all of `frontend/src-tauri/src`. No custom URI scheme handler exists — not applicable.

## 3. Prototype pollution (first-party code)

No first-party deep-merge/deep-clone utilities exist (no `deepMerge`, `deepClone`, ad-hoc `Object.assign` merges, `structuredClone`, `JSON.parse(JSON.stringify(...))`). Settings objects pass through typed `invoke` calls with Serde-typed Rust structs on the receiving end, not merged in JS. No first-party prototype-pollution surface found.

## 4. Dependency vulnerabilities (versions read directly, not audited)

- **Next.js**: `package.json:78` declares `^14.2.25`; `pnpm-lock.yaml:2796` resolves to **14.2.35** — past the fix for CVE-2025-29927 (middleware authorization-bypass). Not directly exploitable here regardless since `next.config.js:7` sets `output: 'export'` (fully static, no Next.js server/middleware at runtime), but good it's patched anyway.
- **React**: `^18.2.0` → locked to **18.3.1**. Current, no outstanding known CVEs.
- **lodash**: `^4.17.21` — final patched release (fixes the historical prototype-pollution CVEs). Nothing outdated; usage should still avoid `_.merge`/`_.mergeWith` on untrusted objects (not observed in this pass).
- **`@blocknote/*` 0.36.0 / prosemirror-\* pinned via `pnpm.overrides`** — recent versions, no known CVE history to cite with confidence.
- **Follow-up needed**: `pnpm audit`/`npm audit` couldn't run (no `node_modules`) — run once install is permitted, especially for the unused-but-present `@tiptap/*`/`@remirror/*` tree, which still resolves in the lockfile and would still surface in an audit despite being dead code.

## 5. CSP

Re-confirms Phase 1/2 (`tauri.conf.json:26-31`): `default-src 'self'`, `style-src 'self' 'unsafe-inline'`, `img-src 'self' asset: https://asset.localhost data:`, `connect-src 'self' http://localhost:11434 http://localhost:5167 http://localhost:8178 https://api.ollama.ai`. No `script-src` override → scripts fall back to `default-src 'self'` — **no `'unsafe-eval'`, no `'unsafe-inline'` for scripts.** No `dangerousDisableAssetCspModification`/`dangerousRemoteDomainIpcAccess` anywhere (full 121-line file checked).

- **CSP scope**: `next.config.js:7` (`output: 'export'`) means the app is a fully static bundle in `frontend/out/`, loaded directly by the Tauri webview (`build.frontendDist: "../out"`) — **no separate Next.js server at runtime**. The Tauri-injected CSP is the only CSP that applies, and covers 100% of the shipped app (dev mode's `next dev` at `devUrl: http://localhost:3118` is outside this CSP's reach, but that's dev-only, not shipped behavior).
- `https://api.ollama.ai` in `connect-src` remains dormant/unused (Phase 2 §3) — still true, no code calls it.

## 6. CORS (frontend-side implications)

No direct `fetch()` calls exist anywhere in `frontend/src` (the one grep hit, `refetch()` in `usePaginatedTranscripts.ts:192`, is unrelated). All network-shaped operations from the webview go through `invoke(...)` to Rust commands, which make the actual HTTP calls server-side (`reqwest`, not subject to browser CORS). `SidebarProvider.tsx:121-122` sets `serverAddress`/`transcriptServerAddress` state but these values are only read for display/config, never used in a `fetch`/`XMLHttpRequest`/`WebSocket` call in `frontend/src`. **The frontend JS has no direct browser-CORS exposure** to the local backend; the Python/FastAPI backend's CORS policy is irrelevant to the webview's own request path since it never talks to it directly via `fetch`.

## 7. Local API access via `invoke(...)`

Inventory of `invoke()` sites (~36 files) that pass sensitive data or paths:
- **API keys**: `ModelSettingsModal.tsx:534,552,570` (`get_openai_models`/`get_anthropic_models`/`get_groq_models`, `{apiKey: key}`), `:208/:273` (`api_get_api_key`), `TranscriptSettings.tsx:45` (`api_get_transcript_api_key`) — keys flow JS→Rust only for user-triggered calls; Rust side never logs them (zero hits for `api_key` near `info!`/`debug!`/`warn!`/`println!`).
- **Filesystem paths**: `useImportAudio.ts:191,229`, `useTranscriptRecovery.ts:203` (`cleanup_checkpoints`, `{meetingFolder}`), `OnboardingContext.tsx:189,203` (`import_and_initialize_database`, `{legacyDbPath}`).
- **Rust-side validation**: `start_import_audio_command` → `run_import` (`audio/import.rs:255-344`) checks `source.exists()` and builds the destination via `create_meeting_folder()`, which calls `sanitize_filename()` (`audio_processing.rs:14-24`) before joining to the base recordings folder — path traversal via meeting title is mitigated (see §10). `cleanup_checkpoints` (`audio/incremental_saver.rs:376-381`) does `PathBuf::from(&meeting_folder).join(".checkpoints")` then `remove_dir_all` with **no validation that `meeting_folder` is under the app's recordings directory** — trusts the caller. Current caller only passes back a folder path the Rust side itself generated earlier in the flow, so exploitability is low today, but no server-side bounds-check exists if that assumption is ever violated by a future caller.
- `api_test_custom_openai_connection` (`api/api.rs:1272-1290`) validates the endpoint starts with `http://`/`https://` before use — reasonable minimal check for a deliberately user-configured, open-by-design feature (not a strict SSRF allowlist, but appropriate given the feature's purpose).

## 8. IPC permissions vs. actual usage

`tauri.conf.json:45-76` grants `fs:default`, `fs:allow-read-file`, `fs:read-all`, `fs:write-all`, `fs:allow-app-read/write`, `fs:allow-download-read/write`, scoped to `$APPDATA/*`. **Cross-referenced against actual webview JS: `@tauri-apps/plugin-fs` is never imported anywhere in `frontend/src`** (zero hits for `plugin-fs`, `readTextFile`, `writeTextFile`, `BaseDirectory`). Every file operation the UI performs is mediated instead by narrow, purpose-built `#[tauri::command]` handlers with no user-controlled arbitrary path:
- `open_database_folder`, `open_models_folder`, `open_recordings_folder` — all open a fixed, app-computed directory; no path parameter from JS.
- `import_and_initialize_database` — `legacy_db_path` comes from a native file-picker dialog result, not free-typed text.
- `start_import_audio_command`/`validate_audio_file_command` — `sourcePath` also comes from a native file dialog/drag-drop.

**This means the broad `fs:read-all`/`fs:write-all` grant is over-provisioned relative to actual frontend usage — none of it is exercised by JS today.** The residual risk: if an XSS/JS-injection bug is ever introduced (e.g. via the dead-but-present `dangerouslySetInnerHTML` in §1, or a future one), injected script could call the raw `fs` plugin (`invoke('plugin:fs|read_file', {path: ...})`) directly against `$APPDATA/*`, bypassing all the narrow Rust command validation, since the capability is already granted at the webview level. **Recommend narrowing the capability set to only what's actually needed and dropping `fs:read-all`/`fs:write-all`/`fs:default`**, consistent with Phase 1's flag.

## 9. Deep links / custom protocol handlers

No `tauri-plugin-deep-link` dependency, no `register_uri_scheme_protocol` call anywhere. Non-issue for this app as currently built.

## 10. File import/export & path traversal

- **README (`README.md:232`) advertises "PDF, DOCX, and Markdown exports"**, but no such feature exists in `frontend/src` — no `.pdf`, `jsPDF`, `html2pdf`, `saveAs`, or any Rust `pdf`/`docx` export command found. The only implemented "export" is **clipboard copy** (`hooks/meeting-details/useCopyOperations.ts:93,181` — `navigator.clipboard.writeText`), which never touches the filesystem and has no filename-construction step. README claim appears aspirational/stale relative to this codebase — a doc/feature mismatch worth flagging to the team, not a security bug.
- **Meeting-title-to-filename path**: confirmed safe. `create_meeting_folder()` (`audio_processing.rs:35-56`) always runs the title through `sanitize_filename()` first — path separators, `:`, `*`, `?`, `"`, `<`, `>`, `\|`, and control characters are replaced with `_`, so a title like `../../../etc/passwd` becomes a literal subfolder name, not a traversal. Used consistently by both live-recording (`recording_saver.rs:229`) and import (`import.rs:344`) flows.
- **Imported files parsed**: audio import only (no document/transcript-file import found). `validate_audio_file`/`decode_audio_file_with_progress` (`audio/import.rs:957-960`) operate on a native-dialog/drag-drop path, parsed via `symphonia`/`ffmpeg` (native decoders) — outside this frontend-focused pass's scope, flag for the Rust/native-code phases if not already covered (Phase 5 handles Rust-side deserialization concerns).

## 11. Tauri-specific dangerous config flags

Checked all of `tauri.conf.json` (121 lines) and grepped `frontend/src-tauri/src` for `dangerousRemoteDomainIpcAccess`, `dangerousDisableAssetCspModification`, `withGlobalTauri`: **none present anywhere.** No dangerous security-relevant Tauri config flags enabled.

## 12. postinstall/lifecycle scripts / typosquats

Cross-referenced against Phase 1 §7 (confirmed none) and re-checked directly — no `postinstall`/`preinstall`/`prepare` keys. Scanned the dependency list for typosquatting: all package names are standard, correctly-scoped packages (`@tauri-apps/*`, `@radix-ui/*`, `@blocknote/*`, `@remirror/*`, `@tiptap/*`, `lodash`, `next`, `react`, `zod`, `react-hook-form`, etc.) — no lookalike/typosquat names identified.

---

## Summary of actionable items

1. Delete or sanitize `frontend/src/app/notes/[id]/page.tsx:174` (`dangerouslySetInnerHTML` on unescaped text) — currently dead/unreachable but a risky pattern shipped in the bundle.
2. Remove unused `@tiptap/*`, `@remirror/*`, `react-markdown`, `remark-gfm` dependencies (dead code, needless audit/CVE surface) — or wire them up if actually planned.
3. Narrow `tauri.conf.json:45-76` capabilities — drop `fs:read-all`/`fs:write-all`/`fs:default`, since zero webview JS calls the `fs` plugin; all real file access is already mediated by scoped Rust commands.
4. Add an internal path-containment check to `cleanup_checkpoints` (`audio/incremental_saver.rs:376`) as defense-in-depth, even though current callers don't pass attacker-controlled paths.
5. Reconcile README's PDF/DOCX/Markdown export claim (`README.md:232`) with actual (clipboard-only) implementation.
6. Re-run `pnpm audit`/`npm audit` once `pnpm install` is authorized.
7. Confirm `https://api.ollama.ai` in CSP `connect-src` can be dropped (still unused, per Phase 2).
