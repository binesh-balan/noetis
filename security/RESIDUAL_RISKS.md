# Residual Risks

Everything below is a known, open item after this audit pass. Nothing here has been fixed. Ordered roughly by severity; full evidence is in the linked phase report.

## Carried over from remediated findings

| Risk | Detail | Source |
|---|---|---|
| macOS/Linux API keys still plaintext | `secure_storage.rs`'s DPAPI protection is Windows-only. Documented upgrade path: `keyring` crate (Credential Manager/Keychain/libsecret) once a build environment exists to verify it compiles on every target. | `REMEDIATION_LOG.md` #1 |
| Webview→backend IPC has no auth gate | Documented as an accepted trust model, not eliminated. A webview compromise still reaches `api_get_api_key` and gets the plaintext key back (decrypted server-side before return). | `REMEDIATION_LOG.md` #1, `THREAT_MODEL.md` |
| Neither fix has been compiled | `cargo`/`rustc` are not installed in the audit environment. Both changes were written and manually traced against known-stable APIs and existing codebase patterns, but **have not built or run**. | `REMEDIATION_LOG.md` |
| `meeting_notes` may still orphan on delete | `PRAGMA foreign_keys` is never enabled for the SQLite connection, so the `ON DELETE CASCADE` on `meeting_notes` may not fire. Not touched by the Finding #2 fix. | `security/reports/08-data-protection.md` §4 |
| SQLite journal/WAL may retain deleted bytes | No `PRAGMA secure_delete=ON` or `VACUUM` anywhere in `database/`. Filesystem/hardware-level recovery is out of scope for any application-level fix. | `security/reports/08-data-protection.md` §14 |

## Not yet addressed (repo owner chose to fix #1/#2 first)

| # | Risk | Severity | Source |
|---|---|---|---|
| 3 | No strict offline mode exists — no global flag, no centralized outbound gate (~10 independent HTTP clients), Ollama/custom-OpenAI endpoints accept any URL with no loopback restriction | MEDIUM-HIGH | `security/reports/03-offline-architecture.md` |
| 4 | `open_external_url` hands an unvalidated URL to `cmd /C start` on Windows — latent command-injection primitive on the IPC boundary (not reachable via today's UI) | MEDIUM | `security/reports/04-native-security.md` §4, `05-rust-security.md` §11 |
| 5 | FFmpeg download (build-time and runtime auto-install) has no checksum verification; CWD-relative binary lookup is a hijack vector if the bundled binary is missing | MEDIUM | `security/reports/04-native-security.md` §4,6 |
| 6 | CI leaks a 20-char prefix of the DigiCert signing key to build logs (bypasses secret masking); release pipeline (`build.yml`) has no post-build installer verification though sibling workflows already have the code | MEDIUM | `security/reports/09-supply-chain.md` §2,7 |
| 7 | `Cargo.lock` doesn't reflect the `cpal`/`esaxx-rs` git-fork patches declared in `Cargo.toml` — unclear what's actually built | MEDIUM | `security/reports/01-inventory.md` §1, `05-rust-security.md` |
| 8 | Committed `frontend/vs_buildtools.exe` (4.46MB, confirmed unused/unreferenced); updater endpoint and `Cargo.toml` repository field still point at upstream `Zackriya-Solutions/meeting-minutes` instead of this fork | MEDIUM | `security/reports/01-inventory.md` |
| 9 | `fs:read-all`/`fs:write-all` Tauri capability is over-provisioned — no webview JS actually uses the `fs` plugin; would become a bypass gadget if XSS is ever introduced | LOW-MEDIUM | `security/reports/06-frontend-security.md` §8 |
| 10 | No SBOM, no dependency/secret/SAST scanning (cargo-audit/cargo-deny/clippy/gitleaks/semgrep) anywhere in CI | LOW-MEDIUM | `security/reports/09-supply-chain.md` §8,9 |
| 11 | No cryptographic hash verification on any downloaded model file (Whisper/Parakeet/on-device LLM) — only size heuristics + a 4-byte magic number | LOW-MEDIUM | `security/reports/07-ai-security.md` §1,2,4 |
| 12 | Prompt-injection "ignore embedded instructions" defense exists only in the final-report system prompt, not the per-chunk/combine prompts used for long transcripts | LOW | `security/reports/07-ai-security.md` §7 |
| 13 | No license disclosure for the on-device LLM model sources (Qwen/unsloth, Gemma/bartowski) — Gemma carries usage-restriction terms not surfaced to the user | LOW (compliance, not security) | `security/reports/07-ai-security.md` §6 |
| 14 | Dead code (`lib_old_complex.rs`, `recording_saver_old.rs`, `core-old.rs`, `audio_v2/`) contains the bulk of raw unsafe/static-mut patterns in the tree but doesn't compile — safe to delete, shrinks the audit surface for future reviewers | LOW (housekeeping) | `security/reports/05-rust-security.md` §1, `08-data-protection.md` §5 |
| 15 | `static mut SAMPLE_COUNTER` in `audio/pipeline.rs` — unsynchronized global mutable state, technically UB, low practical impact | LOW | `security/reports/05-rust-security.md` §1,9 |
| 16 | Fragile `format!`-built SQL column names in `setting.rs` — safe today only because the interpolated value always comes from a hardcoded `match` arm; a future regression could reintroduce SQL injection | LOW | `security/reports/05-rust-security.md` §12 |
| 17 | Export in `AISummary/index.tsx` uses a browser-style Blob download instead of the already-available `tauri-plugin-dialog` save dialog | LOW | `security/reports/08-data-protection.md` §13 |
| 18 | No app-created directory has restrictive permissions set — relies entirely on OS defaults | LOW | `security/reports/08-data-protection.md` §10 |
| 19 | Dead `dangerouslySetInnerHTML` sink in unreachable demo code (`app/notes/[id]/page.tsx`) — not a live vulnerability but a risky pattern shipped in the bundle | LOW | `security/reports/06-frontend-security.md` §1 |

## Not verified at all (Phase 10 deferred by repo owner)

- No runtime network capture has been performed — the "no hidden exfiltration" conclusion across Phase 1-9 is a static-analysis result only, not proven by observed traffic.
- Whether the legacy Python backend (binds `0.0.0.0:5167` in some launch paths) is ever auto-started by the shipped Tauri app — static analysis says no (`externalBin` doesn't reference it), but this hasn't been confirmed by actually running the app.
- Whether the two remediated fixes actually compile, pass their tests, or preserve existing functionality (settings UI, meeting deletion) — see `REMEDIATION_LOG.md`.

## SBOM

Not generated. `syft`/`cargo-cyclonedx` are not installed in this environment and were not fetched (network-tool-install was out of scope for a static-only audit pass). `security/SBOM.json` remains outstanding — generate it once a build environment with one of these tools is available.
