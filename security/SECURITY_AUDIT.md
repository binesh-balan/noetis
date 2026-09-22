# Noetis Security Audit — Summary

**Scope**: Full static security review of the Noetis codebase (a Meetily-derived, privacy-first Tauri desktop meeting app), Phases 1-9 of the audit plan, plus remediation of all 18 findings the audit surfaced. Phase 10 (build + runtime network monitoring) was explicitly deferred by the repo owner and has not run. Baseline commit `a2cb62e` ("Release v0.4.1"), work done on branch `security-hardening`.

## 1. Executive summary

No evidence of hidden data exfiltration was found. Every outbound network call in the codebase is either a user-selected cloud LLM provider (gated behind explicit provider choice + API key entry), a documented model/binary download, the Tauri auto-updater, or opt-in/default-off PostHog telemetry with personal-data fields stripped before send. This conclusion rests on static analysis only — it has not been confirmed by runtime traffic capture (Phase 10), which the owner asked to hold off on.

Nine static-analysis reports were produced (`security/reports/01` through `09`), covering repository inventory, network/exfiltration surface, offline-architecture readiness, native/DLL security, Rust-level security review, frontend/desktop security, AI/model supply-chain security, local data protection, and CI/CD supply-chain security. All 18 findings from those reports — 2 HIGH, 6 MEDIUM, 10 LOW/housekeeping — now have a remediation in `security/REMEDIATION_LOG.md`, applied across roughly 15 commits on `security-hardening`. **None of this has been compiled.** No Rust toolchain is installed in this environment; every Rust change was written and manually traced against known-stable APIs and this codebase's own existing conventions, with programmatic checks where possible (hash-string lengths, struct-literal completeness), but a `cargo check`/build pass is required before merging anything. TypeScript changes were verified with `npx tsc --noEmit` (zero errors, Node was available). New/modified CI workflow YAML was validated as syntactically correct YAML.

Several fixes are partial by design rather than by oversight — documented per-item in `security/RESIDUAL_RISKS.md` — where closing a finding fully needed something this environment didn't have: a compiler, a large blind multi-hundred-MB download, or infrastructure (a minisign keypair, a hosted release feed) the repo owner controls.

## 2. Findings by severity

| Severity | Count | Status |
|---|---|---|
| HIGH | 2 | **Remediated** (unverified by compilation) |
| MEDIUM | 6 | **Remediated**, several partially — see `RESIDUAL_RISKS.md` |
| LOW / housekeeping | 10 | **Remediated** |

Full findings, file:line evidence, and severity justification: `security/reports/01-inventory.md` through `09-supply-chain.md`. Full remediation detail per finding: `security/REMEDIATION_LOG.md`.

## 3. Actual code changes

~15 commits on `security-hardening`, roughly in this order (see `git log security-hardening` for the full list with detailed messages):

1. Encrypt API keys at rest (Windows DPAPI, `secure_storage.rs`) + remove orphaned recording files on meeting delete.
2. Remove unused committed `vs_buildtools.exe`; verify FFmpeg downloads against pinned hashes; harden `open_external_url` against command injection.
3. Remove dead code (`lib_old_complex.rs`, `audio_v2/`, etc.); fix an unsynchronized `static mut`; dedupe SQL column-mapping logic.
4. Narrow the Tauri `fs:*` capability grant; restrict recording-folder permissions on Unix; make the prompt-injection defense consistent across summarization stages.
5. Fix a CI secret-prefix log leak; add post-build installer signature verification to the release workflow.
6. Add a new dedicated `security-scan.yml` CI workflow (lint, audit, secret scan, SBOM generation).
7. Disclose third-party model licenses (README + in-app UI note).
8. Pin SHA-256 hashes for every downloadable model where a trustworthy source for one exists.
9. Add Strict Offline Mode, a real DNS-based Ollama loopback check, and a "forget all API keys" action.

## 4. Removed or disabled external integrations

None fully removed. Strict Offline Mode (commit 9 above) adds a user-controlled toggle that blocks cloud LLM providers and the update check when enabled, but no integration was removed outright, and the toggle defaults to **off** — existing behavior is unchanged unless a user opts in. See `security/reports/03-offline-architecture.md` and `RESIDUAL_RISKS.md` for what the toggle does and doesn't cover yet.

## 5. Network connection inventory

See `security/NETWORK_ALLOWLIST.md`, derived from `security/reports/02-network-audit.md`. Not runtime-verified.

## 6. Test results

**None run.** No Rust toolchain is installed in the audit environment (`cargo --version` fails). Every Rust change was manually traced; `secure_storage.rs` ships three unit tests that have never executed. Frontend changes were verified with `npx tsc --noEmit` after every batch of TS/TSX edits (all passed with zero errors). This is the single most important caveat on this entire audit's remediation work — see §7.

## 7. Remaining risks

Full list with detail: `security/RESIDUAL_RISKS.md`. Headline items:

1. **Nothing has been compiled.** Run `cargo check`/`cargo build`/`cargo test` before merging any of this.
2. macOS/Linux still store API keys in plaintext (Windows-only DPAPI protection so far).
3. The webview→backend IPC trust boundary was documented, not closed — a webview compromise still yields plaintext keys.
4. Strict Offline Mode gates summarization and the update check, not the other ~9 independent HTTP clients in the app (model-list calls, downloads, analytics).
5. Parakeet v3 model downloads (non-HuggingFace host) remain size-only, not hash-verified.
6. Phase 10 (runtime network validation) has never run — the "no hidden exfiltration" conclusion is unproven by observed traffic.
7. The new `security-scan.yml` CI workflow has never actually executed (no CI trigger exists to run it in this environment).
8. `security/SBOM.json` still doesn't exist as a file — the new workflow can generate one, but hasn't run yet.

## 8. Exact next steps

1. Install a Rust toolchain and run `cargo check` (at minimum) against `security-hardening`, on both workspace members (`frontend/src-tauri`, `llama-helper`). Fix anything that fails to compile — this is the mandatory gate before anything else below.
2. Manually verify end-to-end: (a) an existing saved API key still works after the upgrade-on-next-save migration; (b) deleting a meeting removes both its DB rows and its recording folder; (c) toggling Strict Offline Mode actually blocks a cloud-provider summarization call and the update check; (d) the export save dialog and the DPAPI roundtrip both work in a real build.
3. Trigger `security-scan.yml` manually once, review its first report, and tighten `continue-on-error` on whichever checks come back clean.
4. Download the SBOM artifact from that run and commit it to `security/SBOM.json`.
5. Decide whether to proceed with Phase 10 (build in an isolated environment, run with synthetic audio, capture network traffic with OS-level monitoring/firewall) — currently deferred.
6. Consider the larger follow-ups noted as out of scope for this pass: a fully centralized outbound HTTP gate, macOS/Linux keychain integration via the `keyring` crate, and hash-pinning Parakeet v3 once its host's trust is confirmed.

## 9. Final acceptance matrix

| Item | Status | Evidence |
|---|---|---|
| No unauthorized outbound connections observed in tested scenarios | **NOT VERIFIED** | No runtime testing performed (Phase 10 deferred) |
| External AI providers removed or explicitly disabled | **PASS (partial)** | Not removed, but Strict Offline Mode now lets a user disable them for the actual summarization path (`network_policy.rs`, `llm_client.rs`); doesn't yet gate model-listing calls or defaults to off — see `RESIDUAL_RISKS.md` |
| Local inference validated | **NOT VERIFIED** | Static analysis only; Ollama/on-device LLM code paths reviewed but never executed. Ollama endpoint now has a real loopback check (`ollama::resolve_to_loopback_only`) when strict mode is on, but that check itself has never run either |
| DLL loading reviewed | **PASS** | `security/reports/04-native-security.md` §1-3,6 — no unsafe dynamic-loading patterns found in compiled code beyond one documented, hardened case |
| Native dependencies reviewed | **PASS** | `security/reports/01-inventory.md`, `04-native-security.md` — reviewed; FFmpeg download now hash-verified, CWD-hijack fallback removed |
| No unresolved critical vulnerabilities | **PASS** | No CRITICAL-severity finding was identified in this audit |
| No unresolved high vulnerabilities | **FAIL** | Both HIGH findings were remediated but the fixes are unverified by compilation — cannot be marked resolved until built and tested |
| Secrets scan completed | **PASS (partial)** | Manual pattern grep found no hardcoded secrets (`security/reports/01-inventory.md` §13); the new `security-scan.yml` adds an automated gitleaks job, but it has never actually run |
| Dependency audit completed | **NOT VERIFIED** | `cargo-audit` job added to `security-scan.yml`, but not installed in this environment and the workflow has never run |
| Build pipeline reviewed | **PASS** | `security/reports/09-supply-chain.md` — reviewed in full; secret-prefix leak fixed, post-build verification added to the release workflow |
| Offline functionality tested | **NOT VERIFIED** | No runtime testing performed; Strict Offline Mode exists now but has never been exercised |
| Sensitive logging reviewed | **PASS** | `security/reports/08-data-protection.md` §7 — no transcript/API-key content found logged in compiled code |
| Regression tests passed | **NOT VERIFIED** | No test suite has been run in this environment |

**No item above is marked PASS without the underlying static-review evidence cited, and nothing marked NOT VERIFIED or FAIL has been claimed as done. Every PASS in this matrix reflects code review only — "no unresolved high vulnerabilities" stays FAIL until the code that fixes them is actually confirmed to compile.**
