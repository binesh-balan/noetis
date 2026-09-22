# Noetis Security Audit — Summary

**Scope**: Full static security review of the Noetis codebase (a Meetily-derived, privacy-first Tauri desktop meeting app), Phases 1-9 of the audit plan, plus remediation of the two highest-priority findings. Phase 10 (build + runtime network monitoring) was explicitly deferred by the repo owner and has not run. Baseline commit `a2cb62e` ("Release v0.4.1"), work done on branch `security-hardening`.

## 1. Executive summary

No evidence of hidden data exfiltration was found. Every outbound network call in the codebase is either a user-selected cloud LLM provider (gated behind explicit provider choice + API key entry), a documented model/binary download, the Tauri auto-updater, or opt-in/default-off PostHog telemetry with personal-data fields stripped before send. This conclusion rests on static analysis only — it has not been confirmed by runtime traffic capture (Phase 10), which the owner asked to hold off on.

Nine static-analysis reports were produced (`security/reports/01` through `09`), covering repository inventory, network/exfiltration surface, offline-architecture readiness, native/DLL security, Rust-level security review, frontend/desktop security, AI/model supply-chain security, local data protection, and CI/CD supply-chain security. The two highest-severity findings — plaintext API key storage with an ungated IPC retrieval path, and deleted meetings leaving recording files on disk indefinitely — were remediated in this pass (commit `4e3782e`). Neither fix has been compiled or tested, since no Rust toolchain is installed in this environment; both need a build+test pass before merging.

Fourteen additional findings remain open by the repo owner's choice, ranging from MEDIUM (no strict offline mode, an unvalidated-URL command-injection primitive, unverified FFmpeg downloads, a CI secret-prefix log leak) down to LOW (housekeeping items, missing SBOM/CI scanning, dead code). None of these were assessed as requiring immediate action ahead of the two remediated findings.

## 2. Findings by severity

| Severity | Count | Status |
|---|---|---|
| HIGH | 2 | **Remediated** (unverified by compilation — see `REMEDIATION_LOG.md`) |
| MEDIUM | 6 | Open — see `RESIDUAL_RISKS.md` #3-10 |
| LOW / housekeeping | 9 | Open — see `RESIDUAL_RISKS.md` #11-19 |

Full findings, file:line evidence, and severity justification: `security/reports/01-inventory.md` through `09-supply-chain.md`.

## 3. Actual code changes

Commit `4e3782e` on `security-hardening`:

- Added `frontend/src-tauri/src/secure_storage.rs` — Windows DPAPI-based at-rest encryption for API keys, zero new Cargo dependencies.
- Modified `frontend/src-tauri/src/database/repositories/setting.rs` — routes all API key read/write paths (including the `customOpenAIConfig` JSON blob) through `secure_storage`.
- Modified `frontend/src-tauri/src/api/api.rs` — documented the accepted webview/backend IPC trust boundary on the two API-key-retrieval commands; on `api_delete_meeting`, removes the meeting's on-disk recording folder after a successful DB delete.
- Modified `frontend/src-tauri/src/database/repositories/meeting.rs` — `delete_meeting` now returns the deleted meeting's `folder_path` so the caller can clean it up.
- Modified `frontend/src-tauri/src/lib.rs` — registers the new `secure_storage` module.

Full diff and rationale: `git log security-hardening` / `security/REMEDIATION_LOG.md`.

## 4. Removed or disabled external integrations

None. No external integration was removed or disabled in this pass — Phase 3's recommendations (a real "strict offline mode," a centralized outbound gate, loopback-restricting the Ollama endpoint) remain unimplemented by the owner's choice; see `security/reports/03-offline-architecture.md` and `RESIDUAL_RISKS.md` #3.

## 5. Network connection inventory

See `security/NETWORK_ALLOWLIST.md`, derived from `security/reports/02-network-audit.md`. Not runtime-verified.

## 6. Test results

**None run.** No Rust toolchain is installed in the audit environment (`cargo --version` fails). The two remediated fixes include manual code-path tracing against known-stable Win32 APIs and existing codebase conventions, and `secure_storage.rs` ships three unit tests, but nothing has actually been compiled, linted, or executed. This is the single most important caveat on this entire audit's remediation work — see §7.

## 7. Remaining risks

Full list with severity and evidence links: `security/RESIDUAL_RISKS.md`. Headline items:

1. **Both remediated fixes are unverified by compilation.** Run `cargo check`/`cargo build`/`cargo test` before merging.
2. macOS/Linux still store API keys in plaintext (Windows-only protection so far).
3. The webview→backend IPC trust boundary was documented, not closed — a webview compromise still yields plaintext keys.
4. Fourteen open findings from the Phase 1-9 static review, MEDIUM down to LOW severity.
5. Phase 10 (runtime network validation) has never run — the "no hidden exfiltration" conclusion is unproven by observed traffic.
6. No SBOM has been generated (`syft`/`cargo-cyclonedx` not available in this environment).

## 8. Exact next steps

1. Install a Rust toolchain and run `cargo check` (at minimum) against `security-hardening`, on both workspace members (`frontend/src-tauri`, `llama-helper`), before merging. Fix anything that fails to compile.
2. Manually verify: (a) an existing saved API key still works after the upgrade-on-next-save migration; (b) deleting a meeting removes both its DB rows and its recording folder.
3. Decide whether to proceed with Phase 10 (build in an isolated environment, run with synthetic audio, capture network traffic with OS-level monitoring/firewall) — currently deferred.
4. Triage `RESIDUAL_RISKS.md` #3-19 and decide which to remediate next; #3 (no offline mode) and #4 (`open_external_url` injection primitive) are the next-highest-value candidates given the app's privacy-first positioning.
5. Generate `security/SBOM.json` once a build environment with `syft` or `cargo-cyclonedx` is available.

## 9. Final acceptance matrix

| Item | Status | Evidence |
|---|---|---|
| No unauthorized outbound connections observed in tested scenarios | **NOT VERIFIED** | No runtime testing performed (Phase 10 deferred) |
| External AI providers removed or explicitly disabled | **FAIL** | Not removed/disabled — remain available, gated by user opt-in only (`security/reports/03-offline-architecture.md`) |
| Local inference validated | **NOT VERIFIED** | Static analysis only; Ollama/on-device LLM code paths reviewed but never executed |
| DLL loading reviewed | **PASS** | `security/reports/04-native-security.md` §1-3,6 — no unsafe dynamic-loading patterns found in compiled code beyond one documented, hardened case |
| Native dependencies reviewed | **PASS** | `security/reports/01-inventory.md`, `04-native-security.md` — reviewed; several hardening recommendations remain open (`RESIDUAL_RISKS.md` #5) |
| No unresolved critical vulnerabilities | **PASS** | No CRITICAL-severity finding was identified in this audit |
| No unresolved high vulnerabilities | **FAIL** | Both HIGH findings were remediated but the fixes are unverified by compilation — cannot be marked resolved until built and tested |
| Secrets scan completed | **PASS (partial)** | Manual pattern grep found no hardcoded secrets (`security/reports/01-inventory.md` §13); no automated tool (gitleaks) was run — see `RESIDUAL_RISKS.md` #10 |
| Dependency audit completed | **NOT VERIFIED** | `cargo-audit`/`cargo-deny` not installed in this environment; never run (`security/reports/05-rust-security.md`, `09-supply-chain.md` §8) |
| Build pipeline reviewed | **PASS** | `security/reports/09-supply-chain.md` — reviewed in full; several findings remain open (secret-prefix leak, missing post-build verification) |
| Offline functionality tested | **NOT VERIFIED** | No runtime testing performed |
| Sensitive logging reviewed | **PASS** | `security/reports/08-data-protection.md` §7 — no transcript/API-key content found logged in compiled code |
| Regression tests passed | **NOT VERIFIED** | No test suite has been run in this environment |

**No item above is marked PASS without the underlying static-review evidence cited, and nothing marked NOT VERIFIED or FAIL has been claimed as done.**
