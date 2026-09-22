# Residual Risks

All 18 findings from the Phase 1-9 audit now have a remediation (see `REMEDIATION_LOG.md`). This document lists what's still genuinely open after that pass — either because a finding was only partially closed, or because closing it fully needs something this environment doesn't have (a compiler, a large blind download, infrastructure the repo owner controls).

## Not verified by compilation — the biggest open item

**Nothing in this session's ~15 commits of Rust changes has been compiled.** No Rust toolchain is installed in the environment this audit ran in. Every change was traced manually against known-stable APIs (Win32 DPAPI, GitHub/HuggingFace's own published hashes, this codebase's existing conventions) and, where possible, cross-checked programmatically (every struct literal confirmed to include new required fields; every hash string confirmed to be exactly 64 hex characters). Frontend/TypeScript changes were verified with `npx tsc --noEmit` (zero errors) since Node was available. CI workflow YAML was validated as syntactically correct YAML.

**Before merging any of this: run `cargo check` (at minimum) against both workspace members (`frontend/src-tauri`, `llama-helper`).** Fix anything that fails to compile before treating any of the remediations above as done.

## Partial fixes — what each one still leaves open

| Area | What's still open |
|---|---|
| API key encryption | macOS/Linux still store keys in plaintext SQLite (Windows-only DPAPI). Upgrade path: the `keyring` crate (Credential Manager/Keychain/libsecret) once a build environment exists to verify it resolves and compiles on every target. |
| IPC trust boundary | `api_get_api_key`/`api_get_transcript_api_key` still return plaintext keys to any webview JS that calls them — documented as an accepted trust model, not eliminated. A webview compromise (XSS, malicious imported content, compromised dependency) still reaches every stored key. |
| Strict Offline Mode | Gates `generate_summary` (actual summarization traffic) and the update checker only. Does **not** gate ~9 other independent `reqwest::Client` instantiations — opening a cloud provider's tab in Model Settings still fires a model-list request even with strict mode on. A full centralized outbound gate across every HTTP client remains unbuilt. |
| Model hash pinning | Parakeet v3 (`meetily.towardsgeneralintelligence.com`, non-HuggingFace) stays size-only — 2 of its 4 artifacts return S3-style multipart-upload ETags that aren't usable as content hashes, and hashing the other 2 (~670MB) wasn't done blind in this session. Ownership/TLS trust of that host also remains unconfirmed. |
| `Cargo.lock` drift | The `cpal`/`esaxx-rs` patch-vs-lockfile mismatch is documented with a comment, not fixed — hand-editing a machine-generated lockfile without a compiler to verify against was judged unsafe. Run `cargo update -p cpal -p esaxx-rs` (or confirm the patches are dead and remove them) once a toolchain is available. |
| CI/scanning | The new `security-scan.yml` workflow has never actually run (no CI trigger exists to exercise it, and this environment can't run GitHub Actions). All its checks are `continue-on-error: true` since there's no known-clean baseline yet — tighten once a human reviews a first report. |
| Meeting deletion | `meeting_notes` rows may still orphan on delete since `PRAGMA foreign_keys` is never enabled for the SQLite connection. SQLite's WAL/journal may retain deleted bytes until a `VACUUM` — outside what application code can fully guarantee, and consistent with the audit's own instruction not to claim erasure guarantees. |
| Updater endpoint | Still points at upstream `Zackriya-Solutions/meeting-minutes`'s GitHub releases, not a fork-owned feed. Repointing it needs a real minisign keypair and a hosted release — infrastructure the repo owner controls, not a code change. |

## Never fully closed, lower priority

- Windows installer manifest and Linux deb/appimage packaging are bundler-generated at build time with no committed templates to review from source — would need an actual build-output inspection.
- `nice`, `ollama`, `sysctl`, `nvidia-smi`, `osascript`, `open`/`explorer`/`xdg-open` bare-name PATH-relative launches remain (standard desktop-app pattern, low individual severity) — a project-level decision on pinning to absolute system paths wasn't made.
- The legacy Python backend (`backend/`) still binds `0.0.0.0` in some launch paths and has `allow_origins=["*"]` CORS — not touched, since it's marked "archived and unsupported" in the repo's own `CLAUDE.md` and confirmed not bundled with the shipped Tauri app (though that "not bundled" claim itself is still only static-analysis-confirmed, not runtime-verified).

## Not verified at all — Phase 10 deferred

- No runtime network capture has ever been performed. The "no hidden exfiltration" conclusion across every phase report is a static-analysis result, not proven by observed traffic.
- Whether the legacy Python backend is ever auto-started by the shipped Tauri app — static analysis says no, never confirmed by actually running the app.
- Whether any of the remediations in this pass actually work end-to-end at runtime (settings UI round-trips correctly, meeting deletion actually removes files, DPAPI encryption actually round-trips, the strict-offline gate actually blocks a real request) — none of this has been exercised, only read.

## SBOM

`security/SBOM.json` still doesn't exist as a committed file. The new CI workflow can generate one (via `syft`) and upload it as a workflow artifact, but that job has never run — download its output from a workflow run and commit it once the workflow has been triggered at least once.
