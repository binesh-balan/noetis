//! Organization-managed policy, deployed by IT (Intune, GPO file copy, MDM, ...) as a
//! machine-wide JSON file. When present it wins over anything the user configured:
//!
//! ```json
//! {
//!   "summary": {
//!     "endpoint": "https://<resource>.services.ai.azure.com/openai/v1",
//!     "model": "DeepSeek-V4-Flash-0731",
//!     "apiKey": "dpapi:v1:<hex>  (machine-scope DPAPI; see docs/deploy-policy.ps1)",
//!               or a plaintext key, or omit it when a gateway handles auth
//!     "maxTokens": 8192, "temperature": 0.3, "topP": null
//!   },
//!   "transcription": { "provider": "parakeet", "model": "parakeet-tdt-0.6b-v3-int8", "language": "en" },
//!   "modelsDir": "C:\\ProgramData\\Noetis\\models",
//!   "allowModelDownloads": false,
//!   "disableAnalytics": true,
//!   "templatesDir": "\\\\fileserver\\noetis\\templates"
//! }
//! ```
//!
//! `summary` present = every summary uses that OpenAI-compatible endpoint and the
//! summary-model settings are hidden. `transcription` pins provider/model/language and
//! skips onboarding. `modelsDir` is a machine-wide folder of IT-supplied models (same
//! layout as the app's own models folder), used before the per-user one.
//! `allowModelDownloads: false` blocks every model download. `templatesDir` = org-wide
//! template library.
//!
//! A policy file that exists but can't be parsed fails closed (summaries refuse to run)
//! rather than silently falling back to user-chosen providers.

use crate::summary::CustomOpenAIConfig;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedPolicy {
    pub summary: Option<CustomOpenAIConfig>,
    pub transcription: Option<TranscriptionPolicy>,
    pub templates_dir: Option<PathBuf>,
    pub models_dir: Option<PathBuf>,
    pub allow_model_downloads: Option<bool>,
    #[serde(default)]
    pub disable_analytics: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptionPolicy {
    /// "parakeet" or "localWhisper"
    pub provider: String,
    pub model: String,
    /// e.g. "en"; omit for automatic detection
    pub language: Option<String>,
}

// ponytail: path comes from %ProgramData%, which a local user can redirect. This is a
// consistency control for managed fleets, not a boundary against a hostile local user.
pub fn policy_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
        PathBuf::from(base).join("Noetis").join("policy.json")
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/Library/Application Support/Noetis/policy.json")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        PathBuf::from("/etc/noetis/policy.json")
    }
}

pub fn parse(json: &str) -> Result<ManagedPolicy, String> {
    let policy: ManagedPolicy =
        serde_json::from_str(json).map_err(|e| format!("Invalid managed policy: {}", e))?;
    if let Some(s) = &policy.summary {
        if !s.endpoint.starts_with("https://") && !s.endpoint.starts_with("http://") {
            return Err("Invalid managed policy: summary.endpoint must be an http(s) URL".into());
        }
        if s.model.trim().is_empty() {
            return Err("Invalid managed policy: summary.model is required".into());
        }
    }
    if let Some(t) = &policy.transcription {
        if t.provider != "parakeet" && t.provider != "localWhisper" {
            return Err(r#"Invalid managed policy: transcription.provider must be "parakeet" or "localWhisper""#.into());
        }
        if t.model.trim().is_empty() {
            return Err("Invalid managed policy: transcription.model is required".into());
        }
    }
    Ok(policy)
}

/// Decrypts a DPAPI-protected `apiKey` (written by docs/deploy-policy.ps1 with
/// LocalMachine scope, so any account on that PC can decrypt it but the file never holds
/// the key in clear text). Fails closed if it can't be decrypted, e.g. the file was
/// copied from another machine.
// ponytail: machine-scope DPAPI hides the key from casual viewing, not from a local user
// who runs code; keyless Entra ID auth (or a gateway that injects the key) is the upgrade.
fn decrypt_key(mut policy: ManagedPolicy) -> Result<ManagedPolicy, String> {
    if let Some(s) = policy.summary.as_mut() {
        if let Some(key) = s.api_key.as_deref().filter(|k| k.starts_with("dpapi:")) {
            let clear = crate::secure_storage::unprotect(key);
            if clear.is_empty() {
                return Err("Managed policy apiKey could not be decrypted on this machine".into());
            }
            s.api_key = Some(clear);
        }
    }
    Ok(policy)
}

// Read once per process: IT policy changes take effect on next app launch.
static POLICY: Lazy<Result<ManagedPolicy, String>> = Lazy::new(|| {
    let path = policy_path();
    match std::fs::read_to_string(&path) {
        Ok(json) => {
            let policy = parse(&json).and_then(decrypt_key);
            match &policy {
                Ok(_) => log::info!("Loaded managed policy from {:?}", path),
                Err(e) => log::error!("{} ({:?})", e, path),
            }
            policy
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ManagedPolicy::default()),
        Err(e) => Err(format!("Cannot read managed policy {:?}: {}", path, e)),
    }
});

/// The org-enforced summary endpoint, if any. `Err` = policy file broken, refuse to summarize.
pub fn managed_summary() -> Result<Option<CustomOpenAIConfig>, String> {
    POLICY.as_ref().map(|p| p.summary.clone()).map_err(Clone::clone)
}

pub fn templates_dir() -> Option<PathBuf> {
    POLICY.as_ref().ok().and_then(|p| p.templates_dir.clone())
}

pub fn managed_transcription() -> Option<TranscriptionPolicy> {
    POLICY.as_ref().ok().and_then(|p| p.transcription.clone())
}

/// Model downloads are allowed unless the policy forbids them. A broken policy file fails
/// closed (no downloads), like summaries do.
pub fn downloads_allowed() -> bool {
    POLICY.as_ref().map_or(false, |p| p.allow_model_downloads.unwrap_or(true))
}

pub fn require_downloads_allowed() -> Result<(), String> {
    if downloads_allowed() {
        Ok(())
    } else {
        Err("Model downloads are disabled by your organization. Models are provided by IT.".into())
    }
}

pub fn analytics_disabled() -> bool {
    POLICY.as_ref().map_or(true, |p| p.disable_analytics)
}

/// The IT-supplied copy of a model file or folder (path relative to the models folder,
/// e.g. `ggml-base.bin`, `parakeet/<model>`, `diarization/<file>`), if present.
pub fn org_model_path(relative: impl AsRef<std::path::Path>) -> Option<PathBuf> {
    let dir = POLICY.as_ref().ok()?.models_dir.as_ref()?;
    Some(dir.join(relative)).filter(|p| p.exists())
}

pub const MANAGED_ERROR: &str = "Summary model settings are managed by your organization";

/// What the UI needs to render the read-only card. Never includes the API key.
#[tauri::command]
pub fn api_get_managed_policy() -> serde_json::Value {
    match POLICY.as_ref() {
        // Endpoint and model are deliberately not exposed: the managed option stays hidden.
        Ok(p) => serde_json::json!({
            "summaryManaged": p.summary.is_some(),
            "templatesManaged": p.templates_dir.is_some(),
            "transcriptionManaged": p.transcription.is_some(),
            "downloadsAllowed": p.allow_model_downloads.unwrap_or(true),
            "analyticsDisabled": p.disable_analytics,
            "error": null,
        }),
        Err(e) => serde_json::json!({
            "summaryManaged": true, "downloadsAllowed": false, "analyticsDisabled": true, "error": e,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_azure_policy_and_rejects_bad_ones() {
        let p = parse(r#"{"summary":{"endpoint":"https://x.services.ai.azure.com/openai/v1","model":"DeepSeek-V4-Flash-0731","apiKey":"k"},"templatesDir":"C:\\t"}"#).unwrap();
        let s = p.summary.unwrap();
        assert_eq!(s.model, "DeepSeek-V4-Flash-0731");
        assert_eq!(s.api_key.as_deref(), Some("k"));
        assert!(p.templates_dir.is_some());

        assert!(parse("{}").unwrap().summary.is_none());
        assert!(parse(r#"{"summary":{"endpoint":"ftp://x","model":"m"}}"#).is_err());
        assert!(parse(r#"{"summary":{"endpoint":"https://x","model":" "}}"#).is_err());
        assert!(parse(r#"{"sumary":{}}"#).is_err(), "typos must fail closed");
        assert!(parse("not json").is_err());

        let t = parse(r#"{"transcription":{"provider":"parakeet","model":"parakeet-tdt-0.6b-v3-int8","language":"en"},"modelsDir":"C:\\m","allowModelDownloads":false,"disableAnalytics":true}"#).unwrap();
        assert_eq!(t.transcription.unwrap().language.as_deref(), Some("en"));
        assert_eq!(t.allow_model_downloads, Some(false));
        assert!(t.disable_analytics);
        assert!(parse(r#"{"transcription":{"provider":"cloud","model":"x"}}"#).is_err());
        assert!(parse(r#"{"transcription":{"provider":"parakeet","model":""}}"#).is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn decrypts_dpapi_key_and_fails_closed_on_garbage() {
        use super::decrypt_key;
        let enc = crate::secure_storage::protect("sk-secret");
        assert!(enc.starts_with("dpapi:v1:"));
        let json = format!(r#"{{"summary":{{"endpoint":"https://x","model":"m","apiKey":"{enc}"}}}}"#);
        let p = decrypt_key(parse(&json).unwrap()).unwrap();
        assert_eq!(p.summary.unwrap().api_key.as_deref(), Some("sk-secret"));

        let plain = decrypt_key(parse(r#"{"summary":{"endpoint":"https://x","model":"m","apiKey":"k"}}"#).unwrap()).unwrap();
        assert_eq!(plain.summary.unwrap().api_key.as_deref(), Some("k"));

        // Same machine-scope encryption docs/deploy-policy.ps1 performs.
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Add-Type -AssemblyName System.Security; -join ([Security.Cryptography.ProtectedData]::Protect([Text.Encoding]::UTF8.GetBytes('sk-machine'), $null, 'LocalMachine') | % { $_.ToString('x2') })"])
            .output()
            .unwrap();
        let hex = String::from_utf8(out.stdout).unwrap().trim().to_string();
        let json = format!(r#"{{"summary":{{"endpoint":"https://x","model":"m","apiKey":"dpapi:v1:{hex}"}}}}"#);
        let p = decrypt_key(parse(&json).unwrap()).unwrap();
        assert_eq!(p.summary.unwrap().api_key.as_deref(), Some("sk-machine"));

        let bad = parse(r#"{"summary":{"endpoint":"https://x","model":"m","apiKey":"dpapi:v1:00ff"}}"#).unwrap();
        assert!(decrypt_key(bad).is_err());
    }
}
