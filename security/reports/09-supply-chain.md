# Phase 9 — CI/CD and Supply-Chain Security

Static read-only review, baseline `a2cb62e`, branch `security-hardening`. No workflow executed; no scanner binaries installed or fetched. Builds on [01-inventory.md](01-inventory.md) §8 (workflow/trigger/permissions inventory) — not repeated except where directly relevant. Workflows: `build.yml` (715 lines, reusable), `build-windows.yml` (761), `build-devtest.yml` (529), `build-linux.yml` (346), `build-macos.yml` (281), `build-test.yml` (35), `release.yml` (197), `pr-main-check.yml` (65).

## 1. Artifact integrity (signing/checksums/SBOM)

- Windows/macOS artifacts get code-signing only (DigiCert KeyLocker / Apple notarization, per Phase 1 §4). No workflow generates a checksum file or runs `cosign`/`syft` — grep for `sha256|cosign|syft|checksum|cyclonedx|sbom` across all workflows: **zero matches**.
- Linux packages get structural validity checks only, not cryptographic ones: `dpkg-deb --info/--contents` (`build-linux.yml:266-275`), `rpm -qip` (`:277-288`), AppImage `--appimage-help` (`:290-300`). No GPG signing, no checksum sidecar.
- The only integrity mechanism is the Tauri updater `.sig` (minisign) for `.msi`/`.nsis`/`.dmg`/`.app.tar.gz` — covers only those four formats, not `.deb`/`.rpm`/`.AppImage` or the raw `llama-helper`/`ffmpeg` sidecar binaries.
- **Gap**: no SBOM anywhere (see §9).

## 2. Secrets exposure in workflows

Full `secrets.*` inventory: `SM_HOST`, `SM_API_KEY`, `SM_CLIENT_CERT_PASSWORD`, `SM_CODE_SIGNING_CERT_SHA1_HASH`, `SM_CLIENT_CERT_FILE_B64` (DigiCert KeyLocker), `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_ID_PASSWORD`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `NOETIS_RSA_PUBLIC_KEY`, `SUPABASE_URL`, `SUPABASE_ANON_KEY`, `GITHUB_TOKEN`/`GH_TOKEN`. All appear only in `env:`/`with:` contexts for their matching step — no misuse outside expected purpose.

### Finding: partial secret printed to CI logs (bypasses secret masking)

- `build.yml:411`: `Write-Host "  SM_API_KEY: $($env:SM_API_KEY.Substring(0, [Math]::Min(20, $env:SM_API_KEY.Length)))..."`
- Same pattern duplicated at `build-windows.yml:216` and `build-windows.yml:257`.
- GitHub Actions log masking only replaces *exact, full* occurrences of a registered secret value. A 20-character prefix is a different string and is **not masked** — this leaks a 20-char prefix of the DigiCert KeyLocker API key into the Actions log on every signed Windows build (`sign-binaries: true` in `release.yml:108`, `build-test.yml:31`, default in `build-windows.yml`).
- **Fix**: `Write-Host "SM_API_KEY: $(if ($env:SM_API_KEY) {'SET'} else {'NOT SET'})"` — never print any substring of a credential.
- Adjacent lines print `SM_HOST` (a hostname, not secret-classified) in full — low risk, worth trimming for hygiene anyway.
- `build-devtest.yml`'s equivalent "Verify DigiCert Setup" step (`:247-253`) does **not** have this pattern — only calls `smctl --version/keypair ls/healthcheck`. The leak is specific to `build.yml` (the workflow `release.yml` actually calls) and `build-windows.yml`.
- Ruled out explicitly: `echo $APPLE_CERTIFICATE | base64 --decode > certificate.p12` (`build.yml:475`, `build-devtest.yml:182`, `build-macos.yml:116`) is **not** a log leak — stdout is piped straight to `base64 --decode` and redirected to a file, never printed.
- `TAURI_SIGNING_PRIVATE_KEY`/`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` are passed only as `env:` into `tauri-apps/tauri-action@v0` — never echoed or written by repo-owned script code (see §5).

## 3. Untrusted PR execution

Repo-wide grep for `pull_request`/`pull_request_target` across every `.yml`/`.yaml` file: **zero matches**. All 8 workflows trigger only on `workflow_dispatch` or `workflow_call`. `pr-main-check.yml` is misleadingly named (validates version/branch, not a PR) but is itself `workflow_dispatch`-only. Confirms and closes out Phase 1's finding — no path exposes secrets to external-fork PR contributions.

## 4. Dependency pinning — reproducible installs

- **pnpm**: every frontend install uses `pnpm install --frozen-lockfile` consistently (`build.yml:462`, `build-windows.yml:661`, `build-macos.yml:107`, `build-linux.yml:187`, `build-devtest.yml:172`) — CI fails rather than silently drifting from `pnpm-lock.yaml`.
- **cargo**: **no workflow ever passes `--locked` to any `cargo build`** (`build.yml:546,576`, `build-windows.yml:672`, `build-macos.yml:141`, `build-linux.yml:192`, `build-devtest.yml:379`, all for `llama-helper`). The main app build via `tauri-apps/tauri-action@v0` also never passes `--locked`. Without it, a `Cargo.lock` out of sync with `Cargo.toml` gets silently regenerated at build time instead of failing — inconsistent with the pnpm discipline, and directly relevant to Phase 1 §1's flagged `cpal`/`esaxx-rs` patch/lockfile mismatch (CI wouldn't catch that kind of drift today).

## 5. Release signing — minisign updater key end-to-end

- `TAURI_SIGNING_PRIVATE_KEY`/`_PASSWORD` are GitHub Actions repo secrets referenced only in `tauri-apps/tauri-action@v0`'s `env:` block across every build workflow — the intended, documented usage; never written to disk or echoed by repo-owned script code.
- Risk is indirect, not a log leak: (a) anyone with write access to the workflow files could add a step echoing the key — branch-protection on workflow-file changes is the relevant control, out of scope for this static review; (b) `tauri-apps/tauri-action@v0` is pinned to a **major-version tag**, not a SHA — a compromised release of that action could exfiltrate the key it receives directly. This is the highest-value action in the unpinned list (Phase 1 §8) to pin to a commit SHA.
- Updater pubkey (`tauri.conf.json:113-120`) and the private key are consistent with each other. Since the updater endpoint points at upstream `upstream/meeting-minutes` releases (Phase 1 finding #4): **confirm which keypair is actually embedded** — if this fork signs with a different private key than upstream, its builds will fail update-verification against the embedded pubkey unless that pubkey was also regenerated for a fork-owned keypair.

## 6. Build reproducibility

Not verified, and CI does nothing to make it verifiable:
- `build.rs` (Phase 1 §5) downloads FFmpeg with no checksum verification and ONNX Runtime with SHA-256+size verification — CI inherits this unchanged; `actions/cache@v4` (`build.yml:591-601`) caches whatever was fetched but doesn't verify it.
- `dtolnay/rust-toolchain@stable` resolves "stable" at run time — exact rustc version isn't pinned in any workflow, so different runs can use different compilers.
- No `cargo build --locked` (§4) compounds this.
- **Recommend flagging as a known limitation** rather than fixing in this phase, consistent with the audit's own guidance not to claim reproducibility without evidence.

## 7. Installer integrity — post-build verification before upload

Mixed, workflow-dependent:
- `build-linux.yml`: real structural verification (`dpkg-deb`, `rpm -qip`, AppImage smoke test) between build and upload.
- `build-macos.yml`: `spctl -a -vvv "$APP_PATH"` (`:216-217`) — real notarization/Gatekeeper check.
- `build-devtest.yml`: most thorough — `codesign --verify --deep --strict` (`:471-478`) and `Get-AuthenticodeSignature` (`:489-496`), both between build and upload.
- **`build.yml` (the reusable workflow `release.yml` actually calls) and `build-windows.yml` have NO post-build verification of the produced installer.** In `build.yml`, `Build with Tauri` (`:604-629`) goes straight to artifact upload with no check in between. `build-windows.yml`'s `Get-AuthenticodeSignature` calls (`:337`, `:413`) only verify a **synthetic pre-flight test exe**, not the real `.msi`/`.nsis` uploaded at `:720-722`.
- **Recommendation (highest-value, lowest-effort fix in this phase)**: port the existing `Get-AuthenticodeSignature`/`codesign --verify` checks from `build-devtest.yml` into `build.yml`, right after `Build with Tauri` (~`:629`) and before artifact upload (~`:631`). The code already exists in-repo, just in the wrong (test-only) workflow.

## 8. Static/dependency scanning tools

**Installed on this machine** (version-check only, nothing installed in this review): `cargo`/`rustc`/`rustup` — not found; `cargo-audit`, `cargo-deny`, `gitleaks`, `trivy`, `syft`, `semgrep`, `osv-scanner` — all not found. (Since `cargo` itself isn't installed, `clippy`/`fmt` are unavailable here too.)

**CI usage**: grep for `clippy|cargo fmt|cargo-audit|cargo audit|cargo-deny|cargo deny|gitleaks|semgrep|trivy|osv-scanner` across all workflows: **zero matches**. None of these tools run anywhere in CI — no linting, no dependency-vulnerability scanning, no secret scanning, no SAST.

**Recommendations (not applied)**:
- `cargo audit`/`cargo deny` — new job, `workflow_dispatch` or a weekly `schedule:` trigger (matches this repo's all-manual philosophy since there's no `pull_request` trigger to hang it off). `cargo deny check` would directly monitor the `ffmpeg-sidecar branch=main` pinning issue from Phase 1 §1.
- `gitleaks` — highest-value quick win given the §2 finding; run once over full history, then incrementally.
- `cargo clippy`/`cargo fmt --check` — cheap to add right after toolchain setup as a fail-fast lint gate; currently absent entirely.
- `semgrep` — lower priority; scope to `frontend/src` and `backend/` if added.
- `trivy` — most applicable to `backend/docker/` if that "archived" Python backend's container path is still built; low priority pending Phase 1 §3's open question on whether it's genuinely unsupported.
- Cleanest implementation: one new dedicated `security-scan.yml` (or a job appended to `build.yml`), not bolted onto the existing release-build workflows — decouples scan cadence from the release matrix.

## 9. SBOM

**No SBOM anywhere** — confirmed by the same §1 grep (zero hits) and no `sbom.json`/`bom.xml`/`*.cdx.json` files exist in the repo.

- **Recommendation**: `syft` can SBOM the whole polyglot tree (Rust via `Cargo.lock`, JS via `pnpm-lock.yaml`, Python via `backend/requirements.txt`) in one pass (`syft dir:. -o cyclonedx-json`). Alternative: `cargo-cyclonedx`/`cargo-sbom` for the Rust workspace plus a JS-ecosystem equivalent for pnpm.
- `syft` is not installed on this machine and was not fetched (per this phase's no-network-tool-fetch rule) — no SBOM was generated here.
- **Follow-up flagged for Phase 12**: `security/SBOM.json` needs a future session with permission to install `syft` (or equivalent) — out of scope for this read-only pass.

---

## Summary of new findings this phase

1. **Secret-prefix log leak** — `SM_API_KEY` first 20 chars printed via `.Substring(0,20)` in `build.yml:411` and `build-windows.yml:216,257`, bypassing GitHub's exact-match secret masking. **MEDIUM severity** — fix by never printing any substring of a credential.
2. **No `--locked` on any `cargo build`** across all workflows, inconsistent with the `pnpm --frozen-lockfile` discipline used for JS deps.
3. **Release pipeline (`build.yml`) has no post-build installer verification**, while the sibling `build-devtest.yml`/`build-macos.yml`/`build-linux.yml` workflows already contain the relevant checks — an easy port, not new capability to build.
4. **No SBOM, no checksum files, no cosign/syft anywhere** — Phase 12 will need `security/SBOM.json` generated with a tool not available in this session.
5. **No SAST/dependency-audit/secret-scan tooling runs in CI at all.**
6. **Confirmed clean**: no `pull_request`/`pull_request_target` trigger anywhere; the `echo $APPLE_CERTIFICATE | base64 --decode` pattern is safe; `TAURI_SIGNING_PRIVATE_KEY` handling has no direct leak vector beyond the general unpinned-`tauri-action@v0` supply-chain risk already flagged in Phase 1 §8.
