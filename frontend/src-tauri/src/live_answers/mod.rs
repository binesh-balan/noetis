//! Live answer suggestions: while recording, when someone asks the user a question by name (or
//! the hotkey is pressed), draft a short first-person answer with the summary model and show it
//! in a floating card. The user says it themselves; nothing is spoken or sent on their behalf.
//! Spec: docs/superpowers/specs/2026-10-05-live-answers-design.md

mod detect;
mod draft;

use detect::{Detector, Segment};
pub use draft::Draft;
use draft::Trigger;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_store::StoreExt;

const SETTINGS_STORE: &str = "live_answers.json";
pub const DEFAULT_HOTKEY: &str = "CommandOrControl+Shift+Space";
const CARD_LABEL: &str = "answer-card";
const DRAFT_TIMEOUT: Duration = Duration::from_secs(15);
const CARD_LIFETIME: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
}

fn default_hotkey() -> String {
    DEFAULT_HOTKEY.into()
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: false, hotkey: default_hotkey() }
    }
}

/// What the floating card shows; serialised as {"state": "...", ...}.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Card {
    #[default]
    Hidden,
    Thinking,
    Ready { draft: Draft },
    Error { message: String },
}

#[derive(Default)]
struct Live {
    detector: Detector,
    card: Card,
    history: Vec<Draft>,
    warned_no_model: bool,
}

static LIVE: LazyLock<Mutex<Live>> = LazyLock::new(Default::default);
/// Bumped per draft, dismiss and reset; a draft or timer from an older generation is dropped.
/// ponytail: superseded drafts still finish their model call; abort the task if cost matters.
static GEN: AtomicU64 = AtomicU64::new(0);

// --- Settings and hotkey ----------------------------------------------------------------------

pub fn load_settings<R: Runtime>(app: &AppHandle<R>) -> Settings {
    app.store(SETTINGS_STORE)
        .ok()
        .and_then(|s| s.get("settings"))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Registers the hotkey when enabled (and nothing when disabled).
/// ponytail: unregister_all is fine while this is the app's only global shortcut.
pub fn register_hotkey<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let gs = app.global_shortcut();
    gs.unregister_all().map_err(|e| e.to_string())?;
    if settings.enabled {
        let shortcut: Shortcut = settings.hotkey.parse().map_err(|e| format!("Invalid hotkey '{}': {e}", settings.hotkey))?;
        gs.register(shortcut).map_err(|e| format!("Couldn't register {}: {e}", settings.hotkey))?;
    }
    Ok(())
}

#[tauri::command]
pub fn live_answers_get_settings<R: Runtime>(app: AppHandle<R>) -> Settings {
    load_settings(&app)
}

#[tauri::command]
pub fn live_answers_set_settings<R: Runtime>(app: AppHandle<R>, settings: Settings) -> Result<(), String> {
    register_hotkey(&app, &settings)?;
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    store.set("settings", serde_json::to_value(&settings).map_err(|e| e.to_string())?);
    store.save().map_err(|e| e.to_string())
}

// --- Triggers ---------------------------------------------------------------------------------

/// Called by the transcription worker for every finished segment.
pub fn on_segment<R: Runtime>(app: &AppHandle<R>, text: &str, start: f64, end: f64) {
    // The window fills even while disabled, so turning it on mid-meeting has context.
    let names = if load_settings(app).enabled { crate::context::load_profile(app).name_list() } else { Vec::new() };
    let asked = LIVE.lock().unwrap().detector.push(Segment { start, end, text: text.to_string() }, &names, Instant::now());
    if asked {
        start_draft(app.clone(), Trigger::Name);
    }
}

/// Global shortcut pressed.
pub fn on_hotkey<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        if crate::audio::recording_commands::is_recording().await {
            start_draft(app, Trigger::Hotkey);
        }
    });
}

/// Recording stopped: forget this meeting's transcript window and suggestions.
pub fn reset<R: Runtime>(app: &AppHandle<R>) {
    GEN.fetch_add(1, SeqCst);
    *LIVE.lock().unwrap() = Live::default();
    close_card(app);
}

// --- Drafting ---------------------------------------------------------------------------------

fn start_draft<R: Runtime>(app: AppHandle<R>, trigger: Trigger) {
    let gen = GEN.fetch_add(1, SeqCst) + 1;
    let transcript = {
        let mut live = LIVE.lock().unwrap();
        live.card = Card::Thinking;
        live.detector.transcript()
    };
    show_card(&app);
    tauri::async_runtime::spawn(async move {
        let result = tokio::time::timeout(DRAFT_TIMEOUT, draft_answer(&app, &transcript, trigger))
            .await
            .unwrap_or_else(|_| Err("The model took too long to answer".into()));
        if GEN.load(SeqCst) != gen {
            return;
        }
        let card = {
            let mut live = LIVE.lock().unwrap();
            match result {
                Ok(draft) => {
                    live.history.push(draft.clone());
                    let _ = app.emit("live-answer", &draft);
                    Card::Ready { draft }
                }
                Err(e) if e == crate::summary::resolve::NO_MODEL => {
                    if std::mem::replace(&mut live.warned_no_model, true) {
                        Card::Hidden // said once this recording
                    } else {
                        Card::Error { message: "Set up a summary model to get answer suggestions".into() }
                    }
                }
                Err(e) => Card::Error { message: e },
            }
        };
        let hidden = matches!(card, Card::Hidden);
        LIVE.lock().unwrap().card = card;
        if hidden {
            close_card(&app);
        } else {
            hide_card_later(app, gen);
        }
    });
}

async fn draft_answer<R: Runtime>(app: &AppHandle<R>, transcript: &str, trigger: Trigger) -> Result<Draft, String> {
    if transcript.trim().is_empty() {
        return Err("Nothing has been said yet".into());
    }
    let pool = app
        .try_state::<crate::state::AppState>()
        .ok_or("App state not available")?
        .db_manager
        .pool()
        .clone();
    let llm = crate::summary::resolve::configured_llm(app, &pool).await?;
    let profile = crate::context::load_profile(app);
    let background = crate::context::prompt_block(&profile, &crate::context::live_meeting_context());
    let name = profile.name_list().into_iter().next().unwrap_or_else(|| "the user".into());
    let reply = llm
        .complete(&draft::system_prompt(&name), &draft::user_prompt(background.as_deref(), transcript, trigger))
        .await?;
    Ok(draft::parse(&reply))
}

// --- Card window ------------------------------------------------------------------------------

fn show_card<R: Runtime>(app: &AppHandle<R>) {
    if app.get_webview_window(CARD_LABEL).is_some() {
        return;
    }
    let (w, h) = (380.0, 200.0);
    let mut builder = tauri::WebviewWindowBuilder::new(app, CARD_LABEL, tauri::WebviewUrl::App("answer-card.html".into()))
        .title("Noetis answer")
        .inner_size(w, h)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .content_protected(true); // not visible in screen shares
    if let Ok(Some(m)) = app.primary_monitor() {
        let scale = m.scale_factor();
        let (pos, size) = (m.position(), m.size());
        let x = (pos.x as f64 + size.width as f64) / scale - w - 16.0;
        let y = (pos.y as f64 + size.height as f64) / scale - h - 64.0;
        builder = builder.position(x, y);
    }
    if let Err(e) = builder.build() {
        log::warn!("Couldn't open the answer card: {e}");
    }
}

fn close_card<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(CARD_LABEL) {
        let _ = w.close();
    }
}

fn hide_card_later<R: Runtime>(app: AppHandle<R>, gen: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CARD_LIFETIME).await;
        if GEN.load(SeqCst) == gen {
            LIVE.lock().unwrap().card = Card::Hidden;
            close_card(&app);
        }
    });
}

#[tauri::command]
pub fn answer_card_state() -> Card {
    LIVE.lock().unwrap().card.clone()
}

#[tauri::command]
pub fn answer_card_dismiss<R: Runtime>(app: AppHandle<R>) {
    GEN.fetch_add(1, SeqCst);
    LIVE.lock().unwrap().card = Card::Hidden;
    close_card(&app);
}

#[tauri::command]
pub fn live_answer_history() -> Vec<Draft> {
    LIVE.lock().unwrap().history.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_serialises_with_a_state_tag() {
        let ready = Card::Ready { draft: Draft { question: "Q".into(), answer: "A".into(), confident: true } };
        assert_eq!(serde_json::to_value(&ready).unwrap()["state"], "ready");
        assert_eq!(serde_json::to_value(&ready).unwrap()["draft"]["answer"], "A");
        assert_eq!(serde_json::to_value(Card::Thinking).unwrap(), serde_json::json!({"state": "thinking"}));
    }

    #[test]
    fn settings_default_off_with_the_standard_hotkey() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings { enabled: false, hotkey: DEFAULT_HOTKEY.into() });
    }
}
