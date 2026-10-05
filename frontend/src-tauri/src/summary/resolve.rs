//! The model and credentials summaries use, resolved in one place so other features (live
//! answers) make the same call: org policy (Azure, Entra ID token) or the user's settings.

use crate::database::repositories::setting::SettingsRepository;
use crate::summary::llm_client::{generate_summary, LLMProvider};
use sqlx::SqlitePool;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};

pub(crate) const NO_MODEL: &str = "No summary model is set up";

pub(crate) struct ResolvedLlm {
    pub provider: LLMProvider,
    pub model_name: String,
    pub api_key: String,
    pub ollama_endpoint: Option<String>,
    pub custom_openai_endpoint: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub app_data_dir: Option<PathBuf>,
}

/// `provider` is the provider id ("custom-openai", "ollama", "builtin-ai", ...).
pub(crate) async fn resolve_llm<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    provider_id: &str,
    model_name: &str,
) -> Result<ResolvedLlm, String> {
    let provider = LLMProvider::from_str(provider_id)?;

    // Ollama, BuiltInAI and CustomOpenAI don't use the standard API key column.
    let api_key = if matches!(provider, LLMProvider::Ollama | LLMProvider::BuiltInAI | LLMProvider::CustomOpenAI) {
        String::new()
    } else {
        match SettingsRepository::get_api_key(pool, provider_id).await {
            Ok(Some(key)) if !key.is_empty() => key,
            Ok(_) => return Err(format!("API key not found for {}", provider_id)),
            Err(e) => return Err(format!("Failed to retrieve API key for {}: {}", provider_id, e)),
        }
    };

    let ollama_endpoint = if provider == LLMProvider::Ollama {
        match SettingsRepository::get_model_config(pool).await {
            Ok(Some(config)) => config.ollama_endpoint,
            Ok(None) => None,
            Err(e) => {
                log::info!("Failed to retrieve Ollama endpoint: {}, using default", e);
                None
            }
        }
    } else {
        None
    };

    let (mut custom_openai_endpoint, mut custom_key, mut max_tokens, mut temperature, mut top_p) = (None, None, None, None, None);
    if provider == LLMProvider::CustomOpenAI {
        let config = match crate::policy::managed_summary() {
            Ok(None) => SettingsRepository::get_custom_openai_config(pool).await.map_err(|e| e.to_string()),
            managed => managed,
        };
        match config {
            Ok(Some(config)) => {
                log::info!("✓ Using custom OpenAI endpoint: {}", config.endpoint);
                custom_openai_endpoint = Some(config.endpoint);
                custom_key = config.api_key;
                max_tokens = config.max_tokens.map(|t| t as u32);
                temperature = config.temperature;
                top_p = config.top_p;
            }
            Ok(None) => return Err("Custom OpenAI provider selected but no configuration found".into()),
            Err(e) => return Err(format!("Failed to retrieve custom OpenAI config: {}", e)),
        }
    }

    let api_key = if provider == LLMProvider::CustomOpenAI {
        // Keyless org setup: a short-lived Entra ID token instead of an API key.
        if let Some(entra) = crate::policy::managed_entra() {
            let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            crate::entra::access_token(&dir, &entra)
                .await
                .map_err(|e| format!("Sign-in to your organization account is required for summaries: {e:#}"))?
        } else {
            custom_key.unwrap_or_default()
        }
    } else {
        api_key
    };

    Ok(ResolvedLlm {
        provider,
        model_name: model_name.to_string(),
        api_key,
        ollama_endpoint,
        custom_openai_endpoint,
        max_tokens,
        temperature,
        top_p,
        app_data_dir: app.path().app_data_dir().ok(),
    })
}

/// The provider/model a summary would use right now: org policy first, then saved settings.
pub(crate) async fn configured_llm<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool) -> Result<ResolvedLlm, String> {
    let (provider, model) = match crate::policy::managed_summary()? {
        Some(cfg) => ("custom-openai".to_string(), cfg.model),
        None => match SettingsRepository::get_model_config(pool).await.map_err(|e| e.to_string())? {
            Some(s) if !s.provider.is_empty() && !s.model.is_empty() => (s.provider, s.model),
            _ => return Err(NO_MODEL.into()),
        },
    };
    resolve_llm(app, pool, &provider, &model).await
}

impl ResolvedLlm {
    /// One chat call; returns the visible reply text.
    pub(crate) async fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, String> {
        generate_summary(
            &reqwest::Client::new(),
            &self.provider,
            &self.model_name,
            &self.api_key,
            system_prompt,
            user_prompt,
            self.ollama_endpoint.as_deref(),
            self.custom_openai_endpoint.as_deref(),
            self.max_tokens,
            self.temperature,
            self.top_p,
            self.app_data_dir.as_ref(),
            None,
        )
        .await
        .map(|c| c.content)
    }
}
