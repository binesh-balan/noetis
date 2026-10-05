use crate::database::models::{Setting, TranscriptSetting};
use crate::summary::CustomOpenAIConfig;
use sqlx::SqlitePool;

#[derive(serde::Deserialize, Debug)]
pub struct SaveModelConfigRequest {
    pub provider: String,
    pub model: String,
    #[serde(rename = "whisperModel")]
    pub whisper_model: String,
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(rename = "ollamaEndpoint")]
    pub ollama_endpoint: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
pub struct SaveTranscriptConfigRequest {
    pub provider: String,
    pub model: String,
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
}

pub struct SettingsRepository;

/// Maps a summary-provider id to its `settings` table API-key column, or `Ok(None)` for
/// providers that don't need a key. Centralizes what used to be three separately
/// duplicated `match` blocks (in save_api_key/get_api_key/delete_api_key) into one place
/// — see security/reports/05-rust-security.md §12: those duplicates were already safe
/// (every arm maps to a hardcoded literal, with an explicit `Err` fallthrough — `provider`
/// itself never reaches the SQL text), but relied on all three copies staying in sync
/// independently. One function means one place to get right.
fn settings_api_key_column(
    provider: &str,
) -> std::result::Result<Option<&'static str>, sqlx::Error> {
    match provider {
        "openai" => Ok(Some("openaiApiKey")),
        "claude" => Ok(Some("anthropicApiKey")),
        "ollama" => Ok(Some("ollamaApiKey")),
        "groq" => Ok(Some("groqApiKey")),
        "openrouter" => Ok(Some("openRouterApiKey")),
        "builtin-ai" => Ok(None), // No API key needed
        _ => Err(sqlx::Error::Protocol(
            format!("Invalid provider: {}", provider).into(),
        )),
    }
}

/// Same idea as [`settings_api_key_column`], for the `transcript_settings` table.
fn transcript_api_key_column(
    provider: &str,
) -> std::result::Result<Option<&'static str>, sqlx::Error> {
    match provider {
        "localWhisper" => Ok(Some("whisperApiKey")),
        "parakeet" => Ok(None), // Parakeet doesn't need an API key
        "deepgram" => Ok(Some("deepgramApiKey")),
        "elevenLabs" => Ok(Some("elevenLabsApiKey")),
        "groq" => Ok(Some("groqApiKey")),
        "openai" => Ok(Some("openaiApiKey")),
        _ => Err(sqlx::Error::Protocol(
            format!("Invalid provider: {}", provider).into(),
        )),
    }
}

// Transcript providers: localWhisper, deepgram, elevenLabs, groq, openai
// Summary providers: openai, claude, ollama, groq, added openrouter
// NOTE: Handle data exclusion in the higher layer as this is database abstraction layer(using SELECT *)

impl SettingsRepository {
    pub async fn get_model_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<Setting>, sqlx::Error> {
        let setting = sqlx::query_as::<_, Setting>("SELECT * FROM settings LIMIT 1")
            .fetch_optional(pool)
            .await?;
        Ok(setting)
    }

    /// Reads the Strict Offline Mode flag (security/reports/03-offline-architecture.md,
    /// security/RESIDUAL_RISKS.md #3). Defaults to `false` when no settings row exists
    /// yet, matching this app's existing default-permissive behavior.
    pub async fn get_strict_offline_mode(
        pool: &SqlitePool,
    ) -> std::result::Result<bool, sqlx::Error> {
        let value: Option<i64> =
            sqlx::query_scalar("SELECT strictOfflineMode FROM settings WHERE id = '1' LIMIT 1")
                .fetch_optional(pool)
                .await?;
        Ok(value.unwrap_or(0) != 0)
    }

    pub async fn set_strict_offline_mode(
        pool: &SqlitePool,
        enabled: bool,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, strictOfflineMode)
            VALUES ('1', 'openai', 'gpt-4o-2024-11-20', 'large-v3', $1)
            ON CONFLICT(id) DO UPDATE SET
                strictOfflineMode = $1
            "#,
        )
        .bind(enabled as i64)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Clears every stored cloud provider API key (and the custom-OpenAI config, which
    /// embeds its own key) in one call. Addresses the "no UI action to purge stored
    /// credentials" gap noted in security/reports/03-offline-architecture.md §1.
    pub async fn forget_all_api_keys(pool: &SqlitePool) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE settings SET
                openaiApiKey = NULL,
                anthropicApiKey = NULL,
                ollamaApiKey = NULL,
                groqApiKey = NULL,
                openRouterApiKey = NULL,
                geminiApiKey = NULL,
                customOpenAIConfig = NULL
            WHERE id = '1'
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            UPDATE transcript_settings SET
                whisperApiKey = NULL,
                deepgramApiKey = NULL,
                elevenLabsApiKey = NULL,
                groqApiKey = NULL,
                openaiApiKey = NULL
            WHERE id = '1'
            "#,
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn save_model_config(
        pool: &SqlitePool,
        provider: &str,
        model: &str,
        whisper_model: &str,
        ollama_endpoint: Option<&str>,
    ) -> std::result::Result<(), sqlx::Error> {
        // Using id '1' for backward compatibility
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, ollamaEndpoint)
            VALUES ('1', $1, $2, $3, $4)
            ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                model = excluded.model,
                whisperModel = excluded.whisperModel,
                ollamaEndpoint = excluded.ollamaEndpoint
            "#,
        )
        .bind(provider)
        .bind(model)
        .bind(whisper_model)
        .bind(ollama_endpoint)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn save_api_key(
        pool: &SqlitePool,
        provider: &str,
        api_key: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        // Custom OpenAI uses JSON config (customOpenAIConfig) instead of a separate API key column
        if provider == "custom-openai" {
            return Err(sqlx::Error::Protocol(
                "custom-openai provider should use save_custom_openai_config() instead of save_api_key()".into(),
            ));
        }

        let api_key_column = match settings_api_key_column(provider)? {
            Some(col) => col,
            None => return Ok(()),
        };

        let protected_key = crate::secure_storage::protect(api_key);
        let query = format!(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, "{}")
            VALUES ('1', 'openai', 'gpt-4o-2024-11-20', 'large-v3', $1)
            ON CONFLICT(id) DO UPDATE SET
                "{}" = $1
            "#,
            api_key_column, api_key_column
        );
        sqlx::query(&query).bind(protected_key).execute(pool).await?;

        Ok(())
    }

    pub async fn get_api_key(
        pool: &SqlitePool,
        provider: &str,
    ) -> std::result::Result<Option<String>, sqlx::Error> {
        // Custom OpenAI uses JSON config - extract API key from there
        if provider == "custom-openai" {
            let config = Self::get_custom_openai_config(pool).await?;
            return Ok(config.and_then(|c| c.api_key));
        }

        let api_key_column = match settings_api_key_column(provider)? {
            Some(col) => col,
            None => return Ok(None),
        };

        let query = format!(
            "SELECT {} FROM settings WHERE id = '1' LIMIT 1",
            api_key_column
        );
        let stored: Option<String> = sqlx::query_scalar(&query).fetch_optional(pool).await?;
        Ok(stored.map(|value| crate::secure_storage::unprotect(&value)))
    }

    pub async fn get_transcript_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<TranscriptSetting>, sqlx::Error> {
        let setting =
            sqlx::query_as::<_, TranscriptSetting>("SELECT * FROM transcript_settings LIMIT 1")
                .fetch_optional(pool)
                .await?;
        Ok(setting)

    }

    pub async fn save_transcript_config(
        pool: &SqlitePool,
        provider: &str,
        model: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO transcript_settings (id, provider, model)
            VALUES ('1', $1, $2)
            ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                model = excluded.model
            "#,
        )
        .bind(provider)
        .bind(model)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn save_transcript_api_key(
        pool: &SqlitePool,
        provider: &str,
        api_key: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        let api_key_column = match transcript_api_key_column(provider)? {
            Some(col) => col,
            None => return Ok(()),
        };

        let protected_key = crate::secure_storage::protect(api_key);
        let query = format!(
            r#"
            INSERT INTO transcript_settings (id, provider, model, "{}")
            VALUES ('1', 'parakeet', '{}', $1)
            ON CONFLICT(id) DO UPDATE SET
                "{}" = $1
            "#,
            api_key_column, crate::config::DEFAULT_PARAKEET_MODEL, api_key_column
        );
        sqlx::query(&query).bind(protected_key).execute(pool).await?;

        Ok(())
    }

    pub async fn get_transcript_api_key(
        pool: &SqlitePool,
        provider: &str,
    ) -> std::result::Result<Option<String>, sqlx::Error> {
        let api_key_column = match transcript_api_key_column(provider)? {
            Some(col) => col,
            None => return Ok(None),
        };

        let query = format!(
            "SELECT {} FROM transcript_settings WHERE id = '1' LIMIT 1",
            api_key_column
        );
        let stored: Option<String> = sqlx::query_scalar(&query).fetch_optional(pool).await?;
        Ok(stored.map(|value| crate::secure_storage::unprotect(&value)))
    }

    pub async fn delete_api_key(
        pool: &SqlitePool,
        provider: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        // Custom OpenAI uses JSON config - clear the entire config
        if provider == "custom-openai" {
            sqlx::query("UPDATE settings SET customOpenAIConfig = NULL WHERE id = '1'")
                .execute(pool)
                .await?;
            return Ok(());
        }

        let api_key_column = match settings_api_key_column(provider)? {
            Some(col) => col,
            None => return Ok(()),
        };

        let query = format!(
            "UPDATE settings SET {} = NULL WHERE id = '1'",
            api_key_column
        );
        sqlx::query(&query).execute(pool).await?;

        Ok(())
    }

    // ===== CUSTOM OPENAI CONFIG METHODS =====

    /// Gets the custom OpenAI configuration from JSON
    ///
    /// # Returns
    /// * `Ok(Some(CustomOpenAIConfig))` - Config exists and is valid JSON
    /// * `Ok(None)` - No config stored
    /// * `Err(sqlx::Error)` - Database error
    pub async fn get_custom_openai_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<CustomOpenAIConfig>, sqlx::Error> {
        use sqlx::Row;

        let row = sqlx::query(
            r#"
            SELECT customOpenAIConfig
            FROM settings
            WHERE id = '1'
            LIMIT 1
            "#
        )
        .fetch_optional(pool)
        .await?;

        match row {
            Some(record) => {
                let config_json: Option<String> = record.get("customOpenAIConfig");

                if let Some(json) = config_json {
                    // Parse JSON into CustomOpenAIConfig
                    let mut config: CustomOpenAIConfig = serde_json::from_str(&json)
                        .map_err(|e| sqlx::Error::Protocol(
                            format!("Invalid JSON in customOpenAIConfig: {}", e).into()
                        ))?;

                    if let Some(key) = config.api_key.as_deref() {
                        config.api_key = Some(crate::secure_storage::unprotect(key));
                    }

                    Ok(Some(config))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Saves the custom OpenAI configuration as JSON
    ///
    /// # Arguments
    /// * `pool` - Database connection pool
    /// * `config` - CustomOpenAIConfig to save (includes endpoint, apiKey, model, maxTokens, temperature, topP)
    ///
    /// # Returns
    /// * `Ok(())` - Config saved successfully
    /// * `Err(sqlx::Error)` - Database or JSON serialization error
    pub async fn save_custom_openai_config(
        pool: &SqlitePool,
        config: &CustomOpenAIConfig,
    ) -> std::result::Result<(), sqlx::Error> {
        // Serialize config to JSON, protecting the embedded API key at rest the same way
        // the plain settings columns are protected (see secure_storage).
        let mut config_value = serde_json::to_value(config)
            .map_err(|e| sqlx::Error::Protocol(
                format!("Failed to serialize config to JSON: {}", e).into()
            ))?;
        if let Some(obj) = config_value.as_object_mut() {
            if let Some(plaintext_key) = obj.get("apiKey").and_then(|v| v.as_str()) {
                let protected = crate::secure_storage::protect(plaintext_key);
                obj.insert("apiKey".to_string(), serde_json::Value::String(protected));
            }
        }
        let config_json = serde_json::to_string(&config_value)
            .map_err(|e| sqlx::Error::Protocol(
                format!("Failed to serialize config to JSON: {}", e).into()
            ))?;

        // Upsert into settings table
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, customOpenAIConfig)
            VALUES ('1', 'custom-openai', $1, 'large-v3', $2)
            ON CONFLICT(id) DO UPDATE SET
                customOpenAIConfig = excluded.customOpenAIConfig
            "#,
        )
        .bind(&config.model)
        .bind(config_json)
        .execute(pool)
        .await?;

        Ok(())
    }
}
