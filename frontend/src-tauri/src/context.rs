//! "About me" (app-wide, local settings store) and per-meeting context (meeting folder),
//! rendered into one background block for model prompts: summaries and live answers.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_store::StoreExt;

pub const MEETING_CONTEXT_FILE: &str = "meeting-context.json";
const PROFILE_STORE: &str = "profile.json";
const MAX_ABOUT: usize = 2000;
const MAX_FIELD: usize = 500;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    #[serde(default)]
    pub about: String,
    /// Name and nicknames as typed, comma-separated: "Alex, AJ".
    #[serde(default)]
    pub names: String,
}

impl Profile {
    /// Distinct non-empty names, in the order typed.
    pub fn name_list(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for n in self.names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if !out.iter().any(|o| o.eq_ignore_ascii_case(n)) {
                out.push(n.to_string());
            }
        }
        out
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MeetingContext {
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub with: String,
    #[serde(default)]
    pub topics: String,
}

impl MeetingContext {
    pub fn is_empty(&self) -> bool {
        [&self.purpose, &self.with, &self.topics].iter().all(|s| s.trim().is_empty())
    }

    fn clipped(&self) -> Self {
        Self { purpose: clip(&self.purpose, MAX_FIELD), with: clip(&self.with, MAX_FIELD), topics: clip(&self.topics, MAX_FIELD) }
    }
}

fn clip(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

/// The shared background block, or None when there is no context at all.
pub fn prompt_block(profile: &Profile, meeting: &MeetingContext) -> Option<String> {
    let mut lines = Vec::new();
    let names = profile.name_list();
    let about = profile.about.trim();
    if !names.is_empty() || !about.is_empty() {
        let mut line = String::from("About the user");
        if !names.is_empty() {
            line.push_str(&format!(" ({})", names.join(", ")));
        }
        if !about.is_empty() {
            line.push_str(&format!(": {about}"));
        }
        lines.push(line);
    }
    let parts: Vec<String> = [("Purpose", &meeting.purpose), ("With", &meeting.with), ("Topics", &meeting.topics)]
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| format!("{k}: {}", v.trim()))
        .collect();
    if !parts.is_empty() {
        lines.push(format!("This meeting: {}", parts.join(" | ")));
    }
    (!lines.is_empty()).then(|| {
        format!(
            "<background>\n{}\n</background>\nUse the background only to understand who is speaking and why. Facts must still come from the transcript.",
            lines.join("\n")
        )
    })
}

/// A summary's user context: the background block, then whatever the user typed.
pub fn with_background(block: Option<String>, custom_prompt: String) -> String {
    match block {
        Some(b) if custom_prompt.trim().is_empty() => b,
        Some(b) => format!("{b}\n\n{custom_prompt}"),
        None => custom_prompt,
    }
}

// --- About me -------------------------------------------------------------------------------

pub fn load_profile<R: Runtime>(app: &AppHandle<R>) -> Profile {
    app.store(PROFILE_STORE)
        .ok()
        .and_then(|s| s.get("profile"))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_profile<R: Runtime>(app: AppHandle<R>) -> Profile {
    load_profile(&app)
}

#[tauri::command]
pub fn set_profile<R: Runtime>(app: AppHandle<R>, profile: Profile) -> Result<(), String> {
    let p = Profile { about: clip(&profile.about, MAX_ABOUT), names: clip(&profile.names, MAX_FIELD) };
    let store = app.store(PROFILE_STORE).map_err(|e| e.to_string())?;
    store.set("profile", serde_json::to_value(&p).map_err(|e| e.to_string())?);
    store.save().map_err(|e| e.to_string())
}

// --- This meeting ---------------------------------------------------------------------------

/// Context for the current (or next) recording, until it is written to the meeting folder.
static LIVE: Mutex<MeetingContext> = Mutex::new(MeetingContext { purpose: String::new(), with: String::new(), topics: String::new() });

pub fn live_meeting_context() -> MeetingContext {
    LIVE.lock().unwrap().clone()
}

/// Missing or unreadable file = no context.
pub fn load_meeting_context(folder: &Path) -> MeetingContext {
    std::fs::read_to_string(folder.join(MEETING_CONTEXT_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_meeting_context_file(folder: &Path, ctx: &MeetingContext) -> std::io::Result<()> {
    std::fs::write(folder.join(MEETING_CONTEXT_FILE), serde_json::to_string_pretty(&ctx.clipped())?)
}

#[tauri::command]
pub fn get_live_meeting_context() -> MeetingContext {
    live_meeting_context()
}

#[tauri::command]
pub async fn set_live_meeting_context(context: MeetingContext) -> Result<(), String> {
    let ctx = context.clipped();
    *LIVE.lock().unwrap() = ctx.clone();
    // A recording already has its folder: keep the file current too.
    if let Ok(Some(folder)) = crate::audio::recording_commands::get_meeting_folder_path().await {
        save_meeting_context_file(Path::new(&folder), &ctx).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Recording stopped: write the live context into its folder, then start fresh.
pub fn finish_recording(folder: Option<&Path>) {
    let ctx = std::mem::take(&mut *LIVE.lock().unwrap());
    if let (Some(folder), false) = (folder, ctx.is_empty()) {
        if let Err(e) = save_meeting_context_file(folder, &ctx) {
            log::warn!("Couldn't save meeting context to {}: {}", folder.display(), e);
        }
    }
}

async fn meeting_folder<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> Result<Option<PathBuf>, String> {
    let pool = app
        .try_state::<crate::state::AppState>()
        .ok_or("App state not available")?
        .db_manager
        .pool()
        .clone();
    let row: Option<(Option<String>,)> = sqlx::query_as("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| r.0).map(PathBuf::from))
}

#[tauri::command]
pub async fn get_meeting_context<R: Runtime>(app: AppHandle<R>, meeting_id: String) -> Result<MeetingContext, String> {
    Ok(meeting_folder(&app, &meeting_id).await?.map(|f| load_meeting_context(&f)).unwrap_or_default())
}

#[tauri::command]
pub async fn save_meeting_context<R: Runtime>(app: AppHandle<R>, meeting_id: String, context: MeetingContext) -> Result<(), String> {
    let folder = meeting_folder(&app, &meeting_id).await?.ok_or("This meeting has no folder to save context in")?;
    save_meeting_context_file(&folder, &context).map_err(|e| e.to_string())
}

/// Background block for a meeting's summary: About me plus that meeting's saved context.
pub async fn summary_block<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> Option<String> {
    let meeting = meeting_folder(app, meeting_id).await.ok().flatten().map(|f| load_meeting_context(&f)).unwrap_or_default();
    prompt_block(&load_profile(app), &meeting)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(about: &str, names: &str) -> Profile {
        Profile { about: about.into(), names: names.into() }
    }

    #[test]
    fn name_list_trims_and_dedups() {
        assert_eq!(profile("", " Alex, AJ ,, alex ").name_list(), vec!["Alex", "AJ"]);
        assert!(profile("", "").name_list().is_empty());
    }

    #[test]
    fn prompt_block_renders_what_is_there() {
        assert_eq!(prompt_block(&Profile::default(), &MeetingContext::default()), None);
        let b = prompt_block(&profile("IT lead at Contoso", "Alex, AJ"), &MeetingContext::default()).unwrap();
        assert!(b.contains("About the user (Alex, AJ): IT lead at Contoso"), "{b}");
        assert!(!b.contains("This meeting"));
        let m = MeetingContext { purpose: "Q4 rollout".into(), with: "".into(), topics: "dates".into() };
        let b = prompt_block(&Profile::default(), &m).unwrap();
        assert!(b.contains("This meeting: Purpose: Q4 rollout | Topics: dates"), "{b}");
        assert!(b.starts_with("<background>") && b.contains("Facts must still come from the transcript"));
    }

    #[test]
    fn with_background_prepends_the_block() {
        assert_eq!(with_background(None, "keep it short".into()), "keep it short");
        assert_eq!(with_background(Some("<b>".into()), "  ".into()), "<b>");
        assert_eq!(with_background(Some("<b>".into()), "keep it short".into()), "<b>\n\nkeep it short");
    }

    #[test]
    fn meeting_context_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("noetis-ctx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_meeting_context(&dir).is_empty());
        let m = MeetingContext { purpose: " Review ".into(), with: "Priya".into(), topics: "x".repeat(600) };
        save_meeting_context_file(&dir, &m).unwrap();
        let back = load_meeting_context(&dir);
        assert_eq!((back.purpose.as_str(), back.with.as_str(), back.topics.len()), ("Review", "Priya", 500));
        std::fs::write(dir.join(MEETING_CONTEXT_FILE), "not json").unwrap();
        assert!(load_meeting_context(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finish_recording_writes_and_clears() {
        let dir = std::env::temp_dir().join(format!("noetis-ctx-finish-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        *LIVE.lock().unwrap() = MeetingContext { purpose: "Standup".into(), ..Default::default() };
        finish_recording(Some(&dir));
        assert_eq!(load_meeting_context(&dir).purpose, "Standup");
        assert!(live_meeting_context().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
