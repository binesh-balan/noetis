//! Organization-managed policy, deployed by IT (Intune, GPO file copy, MDM, ...) as a
//! machine-wide JSON file. When present it wins over anything the user configured:
//!
//! ```json
//! {
//!   "summary": {
//!     "endpoint": "https://<resource>.services.ai.azure.com/openai/v1",
//!     "model": "DeepSeek-V4-Flash-0731",
//!     "apiKey": "<optional; omit when a gateway handles auth>",
//!     "maxTokens": 8192, "temperature": 0.3, "topP": null
//!   },
//!   "templatesDir": "\\\\fileserver\\noetis\\templates"
//! }
//! ```
//!
//! `summary` present = every summary uses that OpenAI-compatible endpoint and the
//! summary-model settings are read-only. `templatesDir` = org-wide template library.
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
    pub templates_dir: Option<PathBuf>,
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
    Ok(policy)
}

// Read once per process: IT policy changes take effect on next app launch.
static POLICY: Lazy<Result<ManagedPolicy, String>> = Lazy::new(|| {
    let path = policy_path();
    match std::fs::read_to_string(&path) {
        Ok(json) => {
            let policy = parse(&json);
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

pub const MANAGED_ERROR: &str = "Summary model settings are managed by your organization";

/// What the UI needs to render the read-only card. Never includes the API key.
#[tauri::command]
pub fn api_get_managed_policy() -> serde_json::Value {
    match POLICY.as_ref() {
        Ok(p) => serde_json::json!({
            "summaryManaged": p.summary.is_some(),
            "endpoint": p.summary.as_ref().map(|s| &s.endpoint),
            "model": p.summary.as_ref().map(|s| &s.model),
            "templatesManaged": p.templates_dir.is_some(),
            "error": null,
        }),
        Err(e) => serde_json::json!({ "summaryManaged": true, "error": e }),
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
    }
}
