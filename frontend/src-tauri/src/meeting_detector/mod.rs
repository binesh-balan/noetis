//! Meeting detection (Windows): notices a call app holding the microphone and offers to
//! record it. `signal` reads who holds the mic; everything here is pure and unit-tested.

#[cfg(target_os = "windows")]
mod signal;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime};

/// ponytail: calibration knobs, checked in real calls (spec Part 1).
pub const POLL: Duration = Duration::from_secs(2);
/// A meeting app must hold the mic this long before it counts (skips quick mic tests).
pub const START_AFTER: Duration = Duration::from_secs(5);
/// No meeting app may hold the mic for this long before the meeting is over (mute/unmute grace).
pub const END_AFTER: Duration = Duration::from_secs(15);

/// Last path segment of a capability-store key (`C:#Program Files#Zoom#bin#Zoom.exe`), lowercased.
fn exe_of(key: &str) -> String {
    key.rsplit('#').next().unwrap_or(key).to_ascii_lowercase()
}

/// Meeting app for a capability-store key: a NonPackaged exe path or a packaged family name.
pub fn app_for_key(key: &str) -> Option<&'static str> {
    const EXES: &[(&str, &str)] = &[
        ("zoom.exe", "Zoom"),
        ("teams.exe", "Teams"),
        ("ms-teams.exe", "Teams"),
        ("slack.exe", "Slack"),
        ("webexmta.exe", "Webex"),
        ("ciscocollabhost.exe", "Webex"),
        ("discord.exe", "Discord"),
    ];
    const PACKAGES: &[(&str, &str)] = &[("msteams_", "Teams"), ("5319275a.whatsappdesktop_", "WhatsApp")];
    let exe = exe_of(key);
    if exe.contains("noetis") {
        return None;
    }
    let lower = key.to_ascii_lowercase();
    EXES.iter()
        .find(|(e, _)| exe == *e)
        .or_else(|| PACKAGES.iter().find(|(p, _)| lower.starts_with(p)))
        .map(|(_, app)| *app)
}

/// Browser exe name (lowercase) for a capability-store key, if it is a browser.
pub fn browser_exe(key: &str) -> Option<&'static str> {
    const BROWSERS: &[&str] = &["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe"];
    let exe = exe_of(key);
    BROWSERS.iter().find(|b| exe == **b).copied()
}

/// Meeting app named by a browser window title, if the title looks like a call.
pub fn meeting_in_title(title: &str) -> Option<&'static str> {
    const TITLES: &[(&str, &str)] = &[
        ("Meet - ", "Google Meet"),
        ("meet.google.com", "Google Meet"),
        ("Microsoft Teams", "Teams"),
        ("Zoom", "Zoom"),
        ("Webex", "Webex"),
        ("Whereby", "Whereby"),
    ];
    TITLES.iter().find(|(t, _)| title.contains(t)).map(|(_, app)| *app)
}

/// A capability-store entry holds the mic while its session has started and not stopped.
pub fn is_holding(start: u64, stop: u64) -> bool {
    start != 0 && stop == 0
}

/// Meeting apps holding the mic. `keys`: (store key, LastUsedTimeStart, LastUsedTimeStop);
/// `windows`: (exe file name lowercase, title) of visible top-level windows.
pub fn meeting_holders(keys: &[(String, u64, u64)], windows: &[(String, String)]) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (key, start, stop) in keys {
        if !is_holding(*start, *stop) {
            continue;
        }
        if let Some(app) = app_for_key(key) {
            out.push(app);
        } else if let Some(browser) = browser_exe(key) {
            if let Some(app) = windows.iter().filter(|(exe, _)| exe == browser).find_map(|(_, t)| meeting_in_title(t)) {
                out.push(app);
            }
        }
    }
    out.dedup();
    out
}

#[derive(Debug, PartialEq)]
pub enum Event {
    Started(&'static str),
    Ended,
}

/// Idle -> in meeting after START_AFTER of continuous holding; back after END_AFTER released.
#[derive(Default)]
pub struct Detector {
    in_meeting: Option<&'static str>,
    holding_since: Option<Instant>,
    released_since: Option<Instant>,
}

impl Detector {
    pub fn tick(&mut self, now: Instant, holders: &[&'static str]) -> Option<Event> {
        match (self.in_meeting, holders.first()) {
            (None, Some(app)) => {
                let since = *self.holding_since.get_or_insert(now);
                if now.duration_since(since) >= START_AFTER {
                    self.in_meeting = Some(app);
                    self.holding_since = None;
                    return Some(Event::Started(app));
                }
                None
            }
            (None, None) => {
                self.holding_since = None;
                None
            }
            (Some(_), Some(_)) => {
                self.released_since = None;
                None
            }
            (Some(_), None) => {
                let since = *self.released_since.get_or_insert(now);
                if now.duration_since(since) >= END_AFTER {
                    self.in_meeting = None;
                    self.released_since = None;
                    return Some(Event::Ended);
                }
                None
            }
        }
    }
}

const PROMPT_LABEL: &str = "meeting-prompt";
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// Set when the detector starts a recording; only those are auto-stopped.
/// ponytail: if the user stops it and starts another during the same call, that one is also
/// auto-stopped at call end. Track the recording id if that bites.
static DETECTOR_OWNED: AtomicBool = AtomicBool::new(false);
static CURRENT_APP: Mutex<Option<&'static str>> = Mutex::new(None);
/// Bumped per prompt so an old prompt's timeout can't close a newer one.
static PROMPT_GEN: AtomicU64 = AtomicU64::new(0);

pub fn meeting_title(app: &str, now: chrono::DateTime<chrono::Local>) -> String {
    format!("{app} meeting, {}", now.format("%b %-d, %H:%M"))
}

/// Starts the background detector (Windows only).
pub fn spawn<R: Runtime>(app: AppHandle<R>) {
    #[cfg(target_os = "windows")]
    tauri::async_runtime::spawn(async move {
        let mut detector = Detector::default();
        loop {
            tokio::time::sleep(POLL).await;
            let holders = match tokio::task::spawn_blocking(signal::meeting_mic_holders).await {
                Ok(Ok(h)) => h,
                Ok(Err(e)) => {
                    log::warn!("Meeting detection disabled: can't read microphone usage ({e})");
                    return;
                }
                Err(e) => {
                    log::warn!("Meeting detection poll failed: {e}");
                    continue;
                }
            };
            match detector.tick(Instant::now(), &holders) {
                Some(Event::Started(name)) => on_started(&app, name).await,
                Some(Event::Ended) => on_ended(&app).await,
                None => {}
            }
        }
    });
    #[cfg(not(target_os = "windows"))]
    let _ = app;
}

async fn on_started<R: Runtime>(app: &AppHandle<R>, name: &'static str) {
    log::info!("Meeting detected: {name}");
    *CURRENT_APP.lock().unwrap() = Some(name);
    if crate::audio::recording_commands::is_recording().await {
        return;
    }
    let mode = crate::audio::recording_preferences::load_recording_preferences(app)
        .await
        .map(|p| p.meeting_detection)
        .unwrap_or_else(|_| "ask".into());
    match mode.as_str() {
        "auto" => {
            start_recording(app, name);
            use tauri_plugin_notification::NotificationExt;
            let _ = app.notification().builder().title("Noetis").body(format!("Recording {name} meeting")).show();
        }
        "ask" => open_prompt(app),
        _ => {}
    }
}

async fn on_ended<R: Runtime>(app: &AppHandle<R>) {
    log::info!("Meeting ended");
    *CURRENT_APP.lock().unwrap() = None;
    close_prompt(app);
    if DETECTOR_OWNED.swap(false, SeqCst) && crate::audio::recording_commands::is_recording().await {
        log::info!("Stopping the recording the detector started");
        crate::tray::stop_recording_flow(app).await;
    }
}

/// Same path as the tray's Start: flag + navigate the main webview to Home, which starts.
fn start_recording<R: Runtime>(app: &AppHandle<R>, name: &'static str) {
    let Some(main) = app.get_webview_window("main") else { return };
    DETECTOR_OWNED.store(true, SeqCst);
    let title = serde_json::to_string(&meeting_title(name, chrono::Local::now())).unwrap_or_else(|_| "\"\"".into());
    let _ = main.eval(&format!(
        "sessionStorage.setItem('autoStartRecording','true');\
         sessionStorage.setItem('autoStartMeetingName',{title});\
         sessionStorage.setItem('autoStartSource','detector');\
         window.location.assign('/')"
    ));
}

fn open_prompt<R: Runtime>(app: &AppHandle<R>) {
    if app.get_webview_window(PROMPT_LABEL).is_some() {
        return;
    }
    let (w, h) = (360.0, 132.0);
    let mut builder = tauri::WebviewWindowBuilder::new(app, PROMPT_LABEL, tauri::WebviewUrl::App("meeting-prompt.html".into()))
        .title("Noetis")
        .inner_size(w, h)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false);
    if let Ok(Some(m)) = app.primary_monitor() {
        let scale = m.scale_factor();
        let (pos, size) = (m.position(), m.size());
        // Bottom-right, clear of the taskbar.
        let x = (pos.x as f64 + size.width as f64) / scale - w - 16.0;
        let y = (pos.y as f64 + size.height as f64) / scale - h - 64.0;
        builder = builder.position(x, y);
    }
    match builder.build() {
        Ok(_) => {
            let gen = PROMPT_GEN.fetch_add(1, SeqCst) + 1;
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(PROMPT_TIMEOUT).await;
                if PROMPT_GEN.load(SeqCst) == gen {
                    close_prompt(&app);
                }
            });
        }
        Err(e) => log::warn!("Couldn't open the meeting prompt: {e}"),
    }
}

fn close_prompt<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(PROMPT_LABEL) {
        let _ = w.close();
    }
}

/// App name for the prompt window ("Zoom"), or None if the meeting already ended.
#[tauri::command]
pub fn meeting_prompt_info() -> Option<String> {
    CURRENT_APP.lock().unwrap().map(str::to_string)
}

#[tauri::command]
pub async fn meeting_prompt_respond<R: Runtime>(app: AppHandle<R>, record: bool) {
    close_prompt(&app);
    let current = *CURRENT_APP.lock().unwrap();
    if let (true, Some(name)) = (record, current) {
        if !crate::audio::recording_commands::is_recording().await {
            start_recording(&app, name);
        }
    }
}

/// Shows the main window, e.g. so a detector-started recording's error is visible.
#[tauri::command]
pub fn reveal_main_window<R: Runtime>(app: AppHandle<R>) {
    crate::tray::focus_main_window(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_title_format() {
        use chrono::TimeZone;
        let t = chrono::Local.with_ymd_and_hms(2026, 9, 6, 14, 5, 0).unwrap();
        assert_eq!(meeting_title("Zoom", t), "Zoom meeting, Sep 6, 14:05");
    }

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn classifies_apps_and_browsers() {
        assert_eq!(app_for_key(r"C:#Users#me#AppData#Roaming#Zoom#bin#Zoom.exe"), Some("Zoom"));
        assert_eq!(app_for_key("MSTeams_8wekyb3d8bbwe"), Some("Teams"));
        assert_eq!(app_for_key(r"C:#Program Files#Noetis#noetis.exe"), None);
        assert_eq!(app_for_key(r"C:#Program Files#Google#Chrome#Application#chrome.exe"), None);
        assert_eq!(browser_exe(r"C:#Program Files#Google#Chrome#Application#chrome.exe"), Some("chrome.exe"));
        assert_eq!(meeting_in_title("Meet - abc-defg-hij - Google Chrome"), Some("Google Meet"));
        assert_eq!(meeting_in_title("YouTube - Google Chrome"), None);
    }

    #[test]
    fn holders_need_active_session_and_meeting_title_for_browsers() {
        let chrome = r"C:#Program Files#Google#Chrome#Application#chrome.exe".to_string();
        let zoom = r"C:#Zoom#bin#Zoom.exe".to_string();
        let keys = vec![(zoom.clone(), 5, 9), (chrome.clone(), 5, 0)];
        let yt = vec![("chrome.exe".to_string(), "YouTube - Google Chrome".to_string())];
        assert!(meeting_holders(&keys, &yt).is_empty(), "stopped zoom + chrome without meeting title");
        let meet = vec![("chrome.exe".to_string(), "Meet - abc - Google Chrome".to_string())];
        assert_eq!(meeting_holders(&keys, &meet), vec!["Google Meet"]);
        assert_eq!(meeting_holders(&[(zoom, 5, 0)], &[]), vec!["Zoom"]);
    }

    #[test]
    fn starts_after_five_seconds_and_ends_after_fifteen() {
        let t0 = Instant::now();
        let mut d = Detector::default();
        assert_eq!(d.tick(t0, &["Zoom"]), None);
        assert_eq!(d.tick(t0 + s(4), &["Zoom"]), None);
        assert_eq!(d.tick(t0 + s(5), &["Zoom"]), Some(Event::Started("Zoom")));
        assert_eq!(d.tick(t0 + s(20), &[]), None);
        assert_eq!(d.tick(t0 + s(34), &[]), None);
        assert_eq!(d.tick(t0 + s(35), &[]), Some(Event::Ended));
    }

    #[test]
    fn short_mic_use_and_mute_blips_are_ignored() {
        let t0 = Instant::now();
        let mut d = Detector::default();
        d.tick(t0, &["Zoom"]);
        assert_eq!(d.tick(t0 + s(3), &[]), None, "released before 5 s");
        assert_eq!(d.tick(t0 + s(6), &["Zoom"]), None, "timer restarted");
        assert_eq!(d.tick(t0 + s(11), &["Zoom"]), Some(Event::Started("Zoom")));
        d.tick(t0 + s(20), &[]);
        assert_eq!(d.tick(t0 + s(30), &["Zoom"]), None, "unmuted inside grace");
        assert_eq!(d.tick(t0 + s(44), &[]), None, "grace restarted");
        assert_eq!(d.tick(t0 + s(59), &[]), Some(Event::Ended));
    }

    #[test]
    fn two_apps_are_one_meeting() {
        let t0 = Instant::now();
        let mut d = Detector::default();
        d.tick(t0, &["Teams"]);
        assert_eq!(d.tick(t0 + s(5), &["Teams", "Zoom"]), Some(Event::Started("Teams")));
        assert_eq!(d.tick(t0 + s(10), &["Zoom"]), None);
        assert_eq!(d.tick(t0 + s(40), &["Zoom"]), None, "still held by Zoom");
    }
}
