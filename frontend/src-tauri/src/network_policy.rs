//! Central "Strict Offline Mode" flag, checked before outbound network requests from the
//! summarization and auto-update-check paths.
//!
//! Backed by the `settings.strictOfflineMode` DB column (see
//! `SettingsRepository::get_strict_offline_mode`/`set_strict_offline_mode`) but cached
//! here as an in-memory atomic, so the hot paths that need to check it — inside
//! `generate_summary`, the update checker — don't need a DB pool threaded through their
//! call chain just for this one flag. Call [`sync_from_db`] once at startup, and
//! [`set_strict_offline`] again any time the setting is changed, so the in-memory copy
//! never drifts from what's persisted.
//!
//! Addresses security/reports/03-offline-architecture.md §2, §5 and
//! security/RESIDUAL_RISKS.md #3: previously no such flag, and no centralized outbound
//! gate of any kind, existed anywhere in this codebase — every integration module built
//! its own independent `reqwest::Client`.

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};

static STRICT_OFFLINE_MODE: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

pub fn is_strict_offline() -> bool {
    STRICT_OFFLINE_MODE.load(Ordering::Relaxed)
}

pub fn set_strict_offline(enabled: bool) {
    STRICT_OFFLINE_MODE.store(enabled, Ordering::Relaxed);
}

/// Loads the persisted setting into the in-memory flag. Call once during app startup,
/// after the database is initialized. Defaults to (and logs a warning, stays at) `false`
/// if the setting can't be read, matching this app's existing default-permissive
/// behavior rather than failing startup over it.
pub async fn sync_from_db(pool: &sqlx::SqlitePool) {
    match crate::database::repositories::setting::SettingsRepository::get_strict_offline_mode(
        pool,
    )
    .await
    {
        Ok(enabled) => set_strict_offline(enabled),
        Err(e) => {
            log::warn!(
                "Failed to load strictOfflineMode setting, defaulting to off: {}",
                e
            );
        }
    }
}
