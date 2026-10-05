# Threat Model

Scope: Noetis, a local-first Tauri desktop app for meeting recording, transcription, and summarization (Noetis-derived). This document summarizes the threat model implied by the Phase 1-9 static audit (`security/reports/`); it has not been validated against a running build (Phase 10 deferred).

## Assets

1. **Meeting audio recordings** (`audio.mp4` per meeting folder)
2. **Meeting transcripts** (SQLite `transcripts` table + `transcripts.json` per-meeting file)
3. **Meeting summaries** (SQLite `summary_processes`/`transcripts.summary`)
4. **Cloud provider API keys** (OpenAI, Anthropic, Groq, OpenRouter, custom-OpenAI) — now protected at rest on Windows (see `REMEDIATION_LOG.md` #1); still plaintext on macOS/Linux
5. **User's local Ollama / on-device LLM outputs** — not separately sensitive, ephemeral

## Trust boundaries

```
┌─────────────────────────────────────────────┐
│  User's machine                              │
│  ┌───────────────┐      ┌─────────────────┐ │
│  │  Tauri webview │◄────►│  Rust backend   │ │
│  │  (Next.js)     │ IPC  │  (single trust  │ │
│  │                │      │   domain — see  │ │
│  └───────────────┘      │   note below)    │ │
│                          └────────┬────────┘ │
│                                   │           │
│                     ┌─────────────┼─────────┐ │
│                     ▼             ▼         │ │
│              SQLite DB      Recordings      │ │
│              (app data dir) folder          │ │
│                     │             │         │ │
└─────────────────────┼─────────────┼─────────┘
                       │             │
        ┌──────────────┘             └── local filesystem, OS-default
        │                                permissions (no app-level hardening,
        ▼                                see security/reports/08 §10)
  Cloud LLM providers (opt-in, user-configured)
  Model CDNs (HuggingFace, project CDN)
  PostHog (opt-in, default off)
  GitHub Releases (auto-updater)
```

**Note on the webview↔backend boundary**: per `security/reports/05-rust-security.md` §13, this is *not* an enforced security boundary today — any JS in the webview can invoke any `#[tauri::command]`, including ones returning plaintext API keys. This is documented as an accepted trust model in `api.rs` (see the SECURITY NOTE comments added on `api_get_api_key`/`api_get_transcript_api_key`), consistent with how Tauri apps are generally designed (the capability/ACL system gates plugin commands, not custom ones). The practical implication: **a webview compromise (XSS, malicious imported content, a compromised npm dependency) is equivalent to a full backend compromise** for anything the backend can do via a registered command.

## Threat actors considered

1. **Remote attacker via cloud LLM provider or model CDN** — a compromised/malicious response from a configured provider. Mitigated: no agentic capability exists downstream of LLM output (`security/reports/07-ai-security.md` §8) — a malicious response can at most produce a bad summary, not execute code or exfiltrate data.
2. **Malicious/adversarial meeting content (prompt injection)** — someone reads adversarial text aloud, or an imported transcript is crafted. Partially mitigated: role separation + delimiters + an explicit "ignore embedded instructions" rule exist, but only in the final-report system prompt, not the per-chunk/combine prompts (`security/reports/07-ai-security.md` §7).
3. **Local malware / another local process with filesystem access** — could read the SQLite DB directly. Partially mitigated on Windows only (Finding #1 remediation); still fully exposed on macOS/Linux and for anything an attacker can read from the unencrypted recordings folder (audio/transcripts are never encrypted at rest on any platform).
4. **Compromised webview (XSS, malicious dependency)** — equivalent to full backend access per the trust-boundary note above. The over-provisioned `fs:read-all`/`fs:write-all` Tauri capability (`security/reports/06-frontend-security.md` §8) would let such a compromise read/write arbitrary files under `$APPDATA/*` even though no legitimate code path uses that capability today.
5. **Network attacker (MITM) during model/binary download** — FFmpeg's build-time and runtime-auto-install downloads have no checksum verification (`security/reports/04-native-security.md` §6); Whisper/Parakeet/on-device-LLM model downloads use only size heuristics + a 4-byte magic number, not a cryptographic hash (`security/reports/07-ai-security.md` §1).
6. **A user who deletes a meeting expecting it gone** — not an adversarial threat, but a broken assumption. Addressed by Finding #2 remediation (on-disk folder now removed on delete), though SQLite's own journal/WAL files may still retain deleted bytes until a `VACUUM` — outside what application code can fully guarantee.

## What is explicitly NOT a threat in this model

- Cloud LLM providers themselves being adversarial by design — out of scope; the user opts in and supplies their own credentials.
- OS-level compromise (kernel exploit, admin-level malware) — no application-level mitigation can meaningfully defend against this.
- Physical access to an unlocked machine — same as above.

## Open items requiring Phase 10 (deferred) to close

- Confirming no unexpected network destinations exist beyond what static analysis found (`security/reports/02-network-audit.md`).
- Confirming the legacy Python backend (which binds `0.0.0.0:5167` in some launch paths) is genuinely never auto-started by the shipped Tauri app, as static analysis suggests but doesn't prove (`security/reports/03-offline-architecture.md` §4).
