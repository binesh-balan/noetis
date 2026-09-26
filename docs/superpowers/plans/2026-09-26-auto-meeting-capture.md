# Auto Meeting Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Noetis detects Zoom/Teams/Meet/etc. calls on Windows, asks to record, auto-stops at call end, and names speakers from remembered voices (plus Zoom/Teams window hints when available), feeding the existing transcript → speakers → summary chain.

**Architecture:** A Rust background detector polls the Windows microphone capability store every 2 s and drives a pure idle/in-meeting state machine; it opens a tiny standalone prompt window and reuses the tray's start/stop paths. Diarization keeps each speaker's centroid embedding in a new `voice_samples` table; later meetings match centroids against known voices. Part 4 adds an overlap-based namer that reads `speaker-hints.json` (produced by per-app readers planned after the UIA spike).

**Tech Stack:** Tauri 2 / Rust (tokio, sqlx SQLite, `winreg`, `windows` 0.61), Next.js 14 static export, TypeScript, shadcn/ui.

**Spec:** `docs/superpowers/specs/2026-09-26-auto-meeting-capture-design.md`

## Global Constraints

- Branch: `enhance/auto-meeting-capture`. Commit per task; messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Everything local: no new network calls, no server, no cloud APIs.
- Detection is Windows-only; on other OSes it compiles to a no-op and the setting is hidden.
- Detector timings: poll **2 s**, start after **5 s** held, end after **15 s** released. Prompt auto-closes after **60 s**.
- Setting `meeting_detection`: `"off" | "ask" | "auto"`, default **`"ask"`**, stored in `RecordingPreferences`.
- Meeting name format: `"{App} meeting, {Mon D, HH:mm}"` (e.g. `Zoom meeting, Sep 26, 14:05`).
- Detector-started recordings are auto-stopped at meeting end; user-started recordings never are.
- `VOICE_MATCH_THRESHOLD = 0.55`; samples need ≥ **20 s** speech. Known voice = label not matching `^Speaker \d+$`.
- Hints: name needs ≥ **60 %** of a speaker's speech and ≥ **10 s**; hints win over voice matching.
- Voiceprints never leave the machine; deleting a meeting deletes its samples.
- Rust tests: run from `frontend/src-tauri` with `cargo test --no-default-features --features platform-default --lib <filter>` (full default features pull GPU builds).
- Frontend checks from `frontend/`: `node_modules/.bin/tsc --noEmit`, `node scripts/check-colors.mjs`, `node scripts/check-contrast.mjs` (`pnpm` is not on PATH).
- Frontend colours: theme tokens only in `src/` (the colour guard fails on raw Tailwind palette classes).
- Match surrounding code style; mark deliberate simplifications with `ponytail:` comments.

## File map

| File | Responsibility |
|---|---|
| `src-tauri/src/meeting_detector/mod.rs` (new) | App/title classification, state machine, loop, prompt window, commands |
| `src-tauri/src/meeting_detector/signal.rs` (new, Windows) | Registry capability store + window titles → meeting apps holding the mic |
| `src-tauri/src/tray.rs` | Extract `stop_recording_flow` shared by tray + detector |
| `src-tauri/src/audio/recording_preferences.rs` | `meeting_detection` field |
| `src-tauri/src/voices.rs` (new) | `voice_samples` storage, matching, labels, voice commands |
| `src-tauri/migrations/20260926000000_add_voice_samples.sql` (new) | Table |
| `src-tauri/src/diarization.rs` | Return centroids; name speakers via hints + voices; save samples; rename updates samples |
| `src-tauri/src/speaker_hints.rs` (new) | `speaker-hints.json` model + overlap naming |
| `src-tauri/src/database/repositories/meeting.rs` | Delete samples with meeting |
| `src-tauri/src/lib.rs` | Module decls, spawn detector, register commands |
| `src-tauri/Cargo.toml` | Windows deps `winreg`, `windows` |
| `src-tauri/examples/uia_dump.rs` (new) | Part 4 spike tool |
| `public/meeting-prompt.html`, `public/meeting-prompt.js` (new) | Standalone prompt page (no app layout: the root layout mounts recording listeners that must not run twice) |
| `src/hooks/useRecordingStart.ts` | Use detector meeting name; reveal window on failure |
| `src/components/RecordingSettings.tsx` | Meeting detection select |
| `src/components/SpeakerSettings.tsx` (new) | Known voices list |
| `src/app/settings/page.tsx` | Speakers tab |

---

### Task 1: Detector core logic (pure)

**Files:**
- Create: `frontend/src-tauri/src/meeting_detector/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (add `pub mod meeting_detector;` after `pub mod entra;`)

**Interfaces:**
- Produces: `app_for_key(&str) -> Option<&'static str>`, `browser_exe(&str) -> Option<&'static str>`, `meeting_in_title(&str) -> Option<&'static str>`, `is_holding(u64, u64) -> bool`, `meeting_holders(&[(String,u64,u64)], &[(String,String)]) -> Vec<&'static str>`, `Detector::tick(&mut self, Instant, &[&'static str]) -> Option<Event>`, `enum Event { Started(&'static str), Ended }`, consts `POLL`, `START_AFTER`, `END_AFTER`.

- [ ] **Step 1: Write the module with failing tests**

```rust
//! Meeting detection (Windows): notices a call app holding the microphone and offers to
//! record it. `signal` reads who holds the mic; everything here is pure and unit-tested.

use std::time::{Duration, Instant};

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

#[cfg(test)]
mod tests {
    use super::*;

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
```

- [ ] **Step 2: Register module and run tests**

Add `pub mod meeting_detector;` to `lib.rs`. Run: `cargo test --no-default-features --features platform-default --lib meeting_detector`
Expected: 5 passed.

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/src/meeting_detector/mod.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(detector): meeting app classification and idle/in-meeting state machine"
```

---

### Task 2: Windows mic-holder signal

**Files:**
- Create: `frontend/src-tauri/src/meeting_detector/signal.rs`
- Modify: `frontend/src-tauri/src/meeting_detector/mod.rs` (add `#[cfg(target_os = "windows")] mod signal;` under the doc comment)
- Modify: `frontend/src-tauri/Cargo.toml` (`[target.'cfg(target_os = "windows")'.dependencies]`)

**Interfaces:**
- Consumes: `meeting_holders`, `is_holding`, `browser_exe` (Task 1).
- Produces: `signal::meeting_mic_holders() -> std::io::Result<Vec<&'static str>>`.

- [ ] **Step 1: Add dependencies** (both already in `Cargo.lock` transitively)

```toml
winreg = "0.55"
windows = { version = "0.61", features = ["Win32_Foundation", "Win32_UI_WindowsAndMessaging", "Win32_System_Threading"] }
```

- [ ] **Step 2: Write `signal.rs`**

```rust
//! Who holds the microphone, from the Windows privacy "capability access" store, plus visible
//! window titles (to tell a Meet tab from other browser mic use).

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible};
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

const STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

fn push_entries(key: &RegKey, out: &mut Vec<(String, u64, u64)>) {
    for name in key.enum_keys().flatten() {
        if name == "NonPackaged" {
            continue;
        }
        if let Ok(sub) = key.open_subkey(&name) {
            let start = sub.get_value::<u64, _>("LastUsedTimeStart").unwrap_or(0);
            let stop = sub.get_value::<u64, _>("LastUsedTimeStop").unwrap_or(0);
            out.push((name, start, stop));
        }
    }
}

/// (key, LastUsedTimeStart, LastUsedTimeStop) for packaged and NonPackaged apps.
pub fn capability_entries() -> std::io::Result<Vec<(String, u64, u64)>> {
    let root = RegKey::predef(HKEY_CURRENT_USER).open_subkey(STORE)?;
    let mut out = Vec::new();
    push_entries(&root, &mut out);
    if let Ok(np) = root.open_subkey("NonPackaged") {
        push_entries(&np, &mut out);
    }
    Ok(out)
}

fn exe_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(|s| s.to_ascii_lowercase())
    }
}

/// (exe file name lowercase, title) for visible, titled top-level windows.
pub fn visible_window_titles() -> Vec<(String, String)> {
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> windows::core::BOOL {
        let out = &mut *(lp.0 as *mut Vec<(String, String)>);
        if IsWindowVisible(hwnd).as_bool() {
            let mut buf = [0u16; 512];
            let n = GetWindowTextW(hwnd, &mut buf);
            if n > 0 {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if let Some(exe) = exe_name(pid) {
                    out.push((exe, String::from_utf16_lossy(&buf[..n as usize])));
                }
            }
        }
        true.into()
    }
    let mut out: Vec<(String, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Meeting apps currently holding the mic. Err only when the store can't be read at all.
pub fn meeting_mic_holders() -> std::io::Result<Vec<&'static str>> {
    let keys = capability_entries()?;
    // Titles only matter when a browser holds the mic; skip the window walk otherwise.
    let browser_holding = keys.iter().any(|(k, s, e)| super::is_holding(*s, *e) && super::browser_exe(k).is_some());
    let windows = if browser_holding { visible_window_titles() } else { Vec::new() };
    Ok(super::meeting_holders(&keys, &windows))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_capability_store() {
        let entries = super::capability_entries().expect("microphone capability store readable");
        assert!(entries.iter().all(|(k, _, _)| !k.is_empty()));
        assert!(!super::visible_window_titles().is_empty());
    }
}
```

If `windows` 0.61 names differ (e.g. `BOOL` lives in `windows::Win32::Foundation`), fix the imports to what the crate exports; keep behaviour identical.

- [ ] **Step 3: Run tests**

Run: `cargo test --no-default-features --features platform-default --lib meeting_detector`
Expected: 6 passed (Task 1's 5 + `reads_the_capability_store`).

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/Cargo.toml Cargo.lock frontend/src-tauri/src/meeting_detector
git commit -m "feat(detector): read microphone holders from the Windows capability store"
```

---

### Task 3: `meeting_detection` setting

**Files:**
- Modify: `frontend/src-tauri/src/audio/recording_preferences.rs` (struct, `Default`, test)
- Modify: `frontend/src/components/RecordingSettings.tsx`

**Interfaces:**
- Produces: `RecordingPreferences.meeting_detection: String` (`"off"|"ask"|"auto"`); TS `meeting_detection?: MeetingDetection`.

- [ ] **Step 1: Failing Rust test** (append to the file; create `#[cfg(test)] mod tests` if absent)

```rust
#[cfg(test)]
mod meeting_detection_tests {
    use super::*;

    #[test]
    fn old_preferences_default_to_ask() {
        let old = r#"{"save_folder":"C:/x","auto_save":true,"file_format":"mp4"}"#;
        let p: RecordingPreferences = serde_json::from_str(old).unwrap();
        assert_eq!(p.meeting_detection, "ask");
        assert_eq!(RecordingPreferences::default().meeting_detection, "ask");
    }
}
```

Run: `cargo test --no-default-features --features platform-default --lib meeting_detection_tests` → FAIL (no field).

- [ ] **Step 2: Add the field**

In `RecordingPreferences` after `preferred_system_device`:

```rust
    /// Windows meeting detection: "off" | "ask" | "auto" (see `meeting_detector`).
    #[serde(default = "default_meeting_detection")]
    pub meeting_detection: String,
```

Add `meeting_detection: default_meeting_detection(),` to `Default`, and:

```rust
fn default_meeting_detection() -> String {
    "ask".to_string()
}
```

Run the test → PASS.

- [ ] **Step 3: Settings UI**

In `RecordingSettings.tsx`:

```tsx
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';

export type MeetingDetection = 'off' | 'ask' | 'auto';
// add to RecordingPreferences interface:
  meeting_detection?: MeetingDetection;
```

Initial state gets `meeting_detection: 'ask'`. Add state + effect:

```tsx
  const [isWindows, setIsWindows] = useState(false);
  useEffect(() => {
    import('@tauri-apps/plugin-os')
      .then(({ platform }) => setIsWindows(platform() === 'windows'))
      .catch(() => setIsWindows(false));
  }, []);

  const handleMeetingDetectionChange = async (value: string) => {
    const prefs = { ...preferences, meeting_detection: value as MeetingDetection };
    setPreferences(prefs);
    try {
      await invoke('set_recording_preferences', { preferences: prefs });
      toast.success('Meeting detection updated');
    } catch (error) {
      toast.error('Failed to save meeting detection', { description: String(error) });
    }
  };
```

Render right after the "Recording Start Notification" block:

```tsx
      {isWindows && (
        <div className="flex items-center justify-between gap-4 p-4 border rounded-lg">
          <div className="flex-1">
            <div className="font-medium">Meeting Detection</div>
            <div className="text-sm text-muted-foreground">
              Notice Zoom, Teams, Meet and other calls using your microphone
            </div>
          </div>
          <Select value={preferences.meeting_detection ?? 'ask'} onValueChange={handleMeetingDetectionChange}>
            <SelectTrigger className="w-44" aria-label="Meeting detection">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="ask">Ask first</SelectItem>
              <SelectItem value="auto">Record automatically</SelectItem>
              <SelectItem value="off">Off</SelectItem>
            </SelectContent>
          </Select>
        </div>
      )}
```

- [ ] **Step 4: Checks** — `node_modules/.bin/tsc --noEmit` and `node scripts/check-colors.mjs` → clean.

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/audio/recording_preferences.rs frontend/src/components/RecordingSettings.tsx
git commit -m "feat(detector): meeting detection setting (off / ask / auto)"
```

---

### Task 4: Detector loop, prompt window, start/stop wiring

**Files:**
- Modify: `frontend/src-tauri/src/tray.rs` (extract `stop_recording_flow`)
- Modify: `frontend/src-tauri/src/meeting_detector/mod.rs` (loop, prompt, commands)
- Modify: `frontend/src-tauri/src/lib.rs` (spawn + register)
- Create: `frontend/public/meeting-prompt.html`, `frontend/public/meeting-prompt.js`
- Modify: `frontend/src/hooks/useRecordingStart.ts`

**Interfaces:**
- Consumes: Task 1 `Detector`/`Event`/`POLL`, Task 2 `signal::meeting_mic_holders`, Task 3 `meeting_detection`.
- Produces: `tray::stop_recording_flow(&AppHandle<R>)` (async), `meeting_detector::spawn(AppHandle<R>)`, commands `meeting_prompt_info() -> Option<String>`, `meeting_prompt_respond(record: bool)`, `reveal_main_window()`, pure `meeting_title(&str, DateTime<Local>) -> String`.

- [ ] **Step 1: Extract the tray stop body**

In `tray.rs`, add:

```rust
/// Stops the active recording and hands off to frontend post-processing
/// (`recording-stop-complete`: save, navigate, speakers, summary). Shared by the tray and
/// the meeting detector.
pub(crate) async fn stop_recording_flow<R: Runtime>(app: &AppHandle<R>) {
    log::info!("Stopping recording...");
    let data_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to get app data dir: {}", e);
            update_tray_menu_async(app).await;
            return;
        }
    };
    let timestamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let save_path = data_dir.join(format!("recording-{}.wav", timestamp));
    let stop_result = crate::audio::recording_commands::stop_recording(
        app.clone(),
        crate::audio::recording_commands::RecordingArgs { save_path: save_path.to_string_lossy().to_string() },
    )
    .await;
    match stop_result {
        Ok(_) => {
            log::info!("Recording stopped successfully");
            if let Err(e) = app.emit("recording-stop-complete", true) {
                log::error!("Failed to emit recording-stop-complete event: {}", e);
            }
        }
        Err(e) => {
            log::error!("Failed to stop recording: {}", e);
            update_tray_menu_async(app).await;
        }
    }
}
```

Replace the duplicated bodies: in `toggle_recording_handler`'s `if crate::is_recording().await { ... }` branch keep `set_tray_state(&app_clone, RecordingState::Stopping);` then `stop_recording_flow(&app_clone).await;`. In `stop_recording_handler`, keep `set_tray_state` + `focus_main_window`, and the spawned task becomes `stop_recording_flow(&app_clone).await;`. Behaviour is unchanged.

- [ ] **Step 2: Failing test for the meeting title**

Add to `meeting_detector` tests:

```rust
    #[test]
    fn meeting_title_format() {
        use chrono::TimeZone;
        let t = chrono::Local.with_ymd_and_hms(2026, 9, 6, 14, 5, 0).unwrap();
        assert_eq!(meeting_title("Zoom", t), "Zoom meeting, Sep 6, 14:05");
    }
```

- [ ] **Step 3: Loop, prompt and commands** (append to `meeting_detector/mod.rs`, above `#[cfg(test)]`)

```rust
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

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
```

- [ ] **Step 4: Wire into `lib.rs`**

In `.setup`, right after the `tray::create_tray` block: `meeting_detector::spawn(_app.handle().clone());`
In `generate_handler!`, after `diarization::api_rename_speaker,`: 

```rust
            meeting_detector::meeting_prompt_info,
            meeting_detector::meeting_prompt_respond,
            meeting_detector::reveal_main_window,
```

Run: `cargo test --no-default-features --features platform-default --lib meeting_detector` → 7 passed (adds `meeting_title_format`).
Run: `cargo check --no-default-features --features platform-default` → no errors.

- [ ] **Step 5: Prompt page** (standalone: no Next layout, so no duplicate recording listeners)

`frontend/public/meeting-prompt.html`:

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>Noetis</title>
<style>
  /* Values mirror src/app/globals.css :root / .dark tokens. */
  :root { --bg: 240 14.3% 98.6%; --fg: 225 8.3% 9.4%; --muted: 229 8.2% 40.6%; --border: 228 9.8% 90%; --primary: 240 60% 59.8%; --primary-fg: 0 0% 100%; --secondary: 240 11.8% 93.3%; }
  :root.dark { --bg: 228 11.6% 8.4%; --fg: 225 8.7% 91%; --muted: 226 5.9% 56.7%; --border: 225 10.5% 14.9%; --primary: 239 87.1% 75.7%; --primary-fg: 225 12.5% 6.3%; --secondary: 225 12.1% 12.9%; }
  html, body { margin: 0; height: 100%; }
  body { font: 13px/1.4 "Segoe UI", system-ui, sans-serif; background: hsl(var(--bg)); color: hsl(var(--fg)); border: 1px solid hsl(var(--border)); box-sizing: border-box; padding: 14px 16px; display: flex; flex-direction: column; gap: 8px; user-select: none; }
  h1 { font-size: 14px; font-weight: 600; margin: 0; }
  p { margin: 0; color: hsl(var(--muted)); font-size: 12px; }
  .row { display: flex; gap: 8px; margin-top: auto; justify-content: flex-end; }
  button { font: inherit; border: 0; border-radius: 6px; padding: 6px 14px; cursor: pointer; }
  #record { background: hsl(var(--primary)); color: hsl(var(--primary-fg)); font-weight: 600; }
  #dismiss { background: hsl(var(--secondary)); color: hsl(var(--fg)); }
  button:focus-visible { outline: 2px solid hsl(var(--primary)); outline-offset: 2px; }
</style>
</head>
<body>
  <h1 id="title">Meeting detected</h1>
  <p id="hint" hidden>Auto-summary is off (Settings → Summary).</p>
  <div class="row">
    <button id="dismiss">Not now</button>
    <button id="record">Record</button>
  </div>
  <script src="/meeting-prompt.js"></script>
</body>
</html>
```

`frontend/public/meeting-prompt.js`:

```js
// Meeting prompt window (opened by the Rust meeting detector).
const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
const stored = (key) => { try { return localStorage.getItem(key); } catch { return null; } };

const theme = stored('noetis-theme');
const dark = theme === 'dark' || (theme !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
document.documentElement.classList.toggle('dark', dark);
if (stored('isAutoSummary') !== 'true') document.getElementById('hint').hidden = false;

invoke('meeting_prompt_info').then((app) => {
  document.getElementById('title').textContent = `🎙 ${app || 'Meeting'} meeting detected`;
});
document.getElementById('record').onclick = () => invoke('meeting_prompt_respond', { record: true });
document.getElementById('dismiss').onclick = () => invoke('meeting_prompt_respond', { record: false });
```

- [ ] **Step 6: `useRecordingStart` uses the detector name and reveals the window on failure**

In the auto-start effect, right after `sessionStorage.removeItem('autoStartRecording');`:

```ts
          const detectorTitle = sessionStorage.getItem('autoStartMeetingName');
          const fromDetector = sessionStorage.getItem('autoStartSource') === 'detector';
          sessionStorage.removeItem('autoStartMeetingName');
          sessionStorage.removeItem('autoStartSource');
          // A detector start happens with the window hidden; surface failures.
          const revealIfDetector = () => { if (fromDetector) invoke('reveal_main_window').catch(() => undefined); };
```

Call `revealIfDetector();` just before `setStatus(RecordingStatus.IDLE);` in the model-not-ready branch, and at the top of the `catch (error)` block (before the `already in progress` check). Replace `const generatedMeetingTitle = generateMeetingTitle();` in this effect with:

```ts
            const generatedMeetingTitle = detectorTitle || generateMeetingTitle();
```

Add `import { invoke } from '@tauri-apps/api/core';` if not already imported.

- [ ] **Step 7: Checks** — `node_modules/.bin/tsc --noEmit`, `node scripts/check-colors.mjs` clean; `cargo check --no-default-features --features platform-default` clean.

- [ ] **Step 8: Commit**

```bash
git add frontend/src-tauri/src/tray.rs frontend/src-tauri/src/meeting_detector/mod.rs frontend/src-tauri/src/lib.rs frontend/public/meeting-prompt.html frontend/public/meeting-prompt.js frontend/src/hooks/useRecordingStart.ts
git commit -m "feat(detector): ask-to-record prompt, auto start/stop via the tray flow"
```

---

### Task 5: Voice sample storage

**Files:**
- Create: `frontend/src-tauri/migrations/20260926000000_add_voice_samples.sql`
- Create: `frontend/src-tauri/src/voices.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (`pub mod voices;` after `pub mod utils;`; register commands)
- Modify: `frontend/src-tauri/src/diarization.rs` (`api_rename_speaker` in a transaction + relabel)
- Modify: `frontend/src-tauri/src/database/repositories/meeting.rs` (delete samples)

**Interfaces:**
- Produces: `voices::{is_placeholder, normalize, Sample, save_meeting_samples, load_profiles, relabel_in_meeting, list_voices, rename_voice, forget_voice, VoiceSummary}`, commands `api_list_voices`, `api_rename_voice(from, to)`, `api_forget_voice(name)`.

- [ ] **Step 1: Migration**

```sql
-- Voice memory: one speaker embedding per speaker per meeting, labelled with the speaker's
-- current name in that meeting. Labels other than "Speaker N" are known voices.
CREATE TABLE IF NOT EXISTS voice_samples (
    meeting_id  TEXT NOT NULL,
    label       TEXT NOT NULL,
    embedding   BLOB NOT NULL,  -- 256 x f32 little-endian, L2-normalised
    speech_secs REAL NOT NULL,
    created_at  TEXT NOT NULL,
    PRIMARY KEY (meeting_id, label),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_voice_samples_label ON voice_samples(label);
```

- [ ] **Step 2: `voices.rs` with failing tests**

```rust
//! Voice memory: speaker embeddings kept per meeting (`voice_samples`), so later meetings
//! can name speakers. A known voice is any label that isn't "Speaker N". Local only.

use serde::Serialize;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::collections::BTreeMap;
use tauri::{AppHandle, Manager, Runtime};

use crate::state::AppState;

pub fn is_placeholder(label: &str) -> bool {
    label.strip_prefix("Speaker ").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

pub struct Sample {
    pub label: String,
    pub embedding: Vec<f32>,
    pub speech_secs: f64,
}

/// Replaces a meeting's samples.
pub async fn save_meeting_samples(pool: &SqlitePool, meeting_id: &str, samples: &[Sample]) -> sqlx::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM voice_samples WHERE meeting_id = ?").bind(meeting_id).execute(&mut *tx).await?;
    for s in samples {
        sqlx::query("INSERT INTO voice_samples (meeting_id, label, embedding, speech_secs, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(meeting_id).bind(&s.label).bind(to_blob(&s.embedding)).bind(s.speech_secs).bind(&now)
            .execute(&mut *tx).await?;
    }
    tx.commit().await
}

/// Known voices as (name, mean embedding, normalised), ignoring one meeting's samples.
pub async fn load_profiles(pool: &SqlitePool, exclude_meeting: &str) -> sqlx::Result<Vec<(String, Vec<f32>)>> {
    let rows = sqlx::query("SELECT label, embedding FROM voice_samples WHERE meeting_id != ?")
        .bind(exclude_meeting)
        .fetch_all(pool)
        .await?;
    let mut sums: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    for r in rows {
        let label: String = r.get("label");
        if is_placeholder(&label) {
            continue;
        }
        let e = from_blob(r.get::<Vec<u8>, _>("embedding").as_slice());
        let sum = sums.entry(label).or_insert_with(|| vec![0.0; e.len()]);
        sum.iter_mut().zip(&e).for_each(|(a, b)| *a += b);
    }
    Ok(sums.into_iter().map(|(k, v)| (k, normalize(v))).collect())
}

async fn sample_row(tx: &mut Transaction<'_, Sqlite>, meeting_id: &str, label: &str) -> sqlx::Result<Option<(Vec<f32>, f64)>> {
    let row = sqlx::query("SELECT embedding, speech_secs FROM voice_samples WHERE meeting_id = ? AND label = ?")
        .bind(meeting_id).bind(label).fetch_optional(&mut **tx).await?;
    Ok(row.map(|r| (from_blob(r.get::<Vec<u8>, _>("embedding").as_slice()), r.get("speech_secs"))))
}

/// Moves a meeting's sample from `from` to `to`; merges (speech-weighted) if `to` exists.
pub async fn relabel_in_meeting(tx: &mut Transaction<'_, Sqlite>, meeting_id: &str, from: &str, to: &str) -> sqlx::Result<()> {
    if from == to {
        return Ok(());
    }
    let Some((a, sa)) = sample_row(tx, meeting_id, from).await? else { return Ok(()) };
    match sample_row(tx, meeting_id, to).await? {
        None => {
            sqlx::query("UPDATE voice_samples SET label = ? WHERE meeting_id = ? AND label = ?")
                .bind(to).bind(meeting_id).bind(from).execute(&mut **tx).await?;
        }
        Some((b, sb)) => {
            let merged = normalize(a.iter().zip(&b).map(|(x, y)| x * sa as f32 + y * sb as f32).collect());
            sqlx::query("UPDATE voice_samples SET embedding = ?, speech_secs = ? WHERE meeting_id = ? AND label = ?")
                .bind(to_blob(&merged)).bind(sa + sb).bind(meeting_id).bind(to).execute(&mut **tx).await?;
            sqlx::query("DELETE FROM voice_samples WHERE meeting_id = ? AND label = ?")
                .bind(meeting_id).bind(from).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, PartialEq)]
pub struct VoiceSummary {
    pub name: String,
    pub meetings: i64,
    pub last_heard: String,
}

pub async fn list_voices(pool: &SqlitePool) -> sqlx::Result<Vec<VoiceSummary>> {
    let rows = sqlx::query("SELECT label, COUNT(*) AS n, MAX(created_at) AS last FROM voice_samples GROUP BY label ORDER BY label COLLATE NOCASE")
        .fetch_all(pool).await?;
    Ok(rows.into_iter()
        .map(|r| VoiceSummary { name: r.get("label"), meetings: r.get("n"), last_heard: r.get("last") })
        .filter(|v| !is_placeholder(&v.name))
        .collect())
}

/// Renames a known voice everywhere: its samples (merging per meeting) and transcript lines.
pub async fn rename_voice(pool: &SqlitePool, from: &str, to: &str) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    let meetings: Vec<String> = sqlx::query_scalar("SELECT meeting_id FROM voice_samples WHERE label = ?")
        .bind(from).fetch_all(&mut *tx).await?;
    for m in meetings {
        relabel_in_meeting(&mut tx, &m, from, to).await?;
    }
    sqlx::query("UPDATE transcripts SET speaker = ? WHERE speaker = ?").bind(to).bind(from).execute(&mut *tx).await?;
    tx.commit().await
}

/// Deletes a voice's samples. Transcripts keep the name.
pub async fn forget_voice(pool: &SqlitePool, name: &str) -> sqlx::Result<u64> {
    Ok(sqlx::query("DELETE FROM voice_samples WHERE label = ?").bind(name).execute(pool).await?.rows_affected())
}

fn pool<R: Runtime>(app: &AppHandle<R>) -> Result<SqlitePool, String> {
    Ok(app.try_state::<AppState>().ok_or("App state not available")?.db_manager.pool().clone())
}

fn valid_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("Speaker name must be 1-64 characters".into());
    }
    Ok(name)
}

#[tauri::command]
pub async fn api_list_voices<R: Runtime>(app: AppHandle<R>) -> Result<Vec<VoiceSummary>, String> {
    list_voices(&pool(&app)?).await.map_err(|e| format!("Failed to list voices: {e}"))
}

#[tauri::command]
pub async fn api_rename_voice<R: Runtime>(app: AppHandle<R>, from: String, to: String) -> Result<(), String> {
    let to = valid_name(&to)?;
    rename_voice(&pool(&app)?, &from, to).await.map_err(|e| format!("Failed to rename voice: {e}"))
}

#[tauri::command]
pub async fn api_forget_voice<R: Runtime>(app: AppHandle<R>, name: String) -> Result<u64, String> {
    forget_voice(&pool(&app)?, &name).await.map_err(|e| format!("Failed to forget voice: {e}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    pub(crate) async fn test_pool() -> SqlitePool {
        // One connection: every new connection to sqlite::memory: is a fresh database.
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for m in ["m1", "m2"] {
            sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, '2026-09-26', '2026-09-26')")
                .bind(m).bind(m).execute(&pool).await.unwrap();
        }
        pool
    }

    fn sample(label: &str, e: [f32; 2], secs: f64) -> Sample {
        Sample { label: label.into(), embedding: normalize(e.to_vec()), speech_secs: secs }
    }

    #[test]
    fn placeholder_labels() {
        assert!(is_placeholder("Speaker 3"));
        assert!(!is_placeholder("Speaker"));
        assert!(!is_placeholder("Speaker X"));
        assert!(!is_placeholder("Priya"));
    }

    #[tokio::test]
    async fn profiles_average_named_samples_and_skip_the_current_meeting() {
        let pool = test_pool().await;
        save_meeting_samples(&pool, "m1", &[sample("Priya", [1.0, 0.0], 30.0), sample("Speaker 2", [0.0, 1.0], 30.0)]).await.unwrap();
        save_meeting_samples(&pool, "m2", &[sample("Priya", [0.0, 1.0], 30.0)]).await.unwrap();
        let p = load_profiles(&pool, "none").await.unwrap();
        assert_eq!(p.len(), 1, "placeholder skipped");
        assert!((p[0].1[0] - p[0].1[1]).abs() < 1e-6, "mean of both samples");
        assert_eq!(load_profiles(&pool, "m2").await.unwrap()[0].1, vec![1.0, 0.0]);
    }

    #[tokio::test]
    async fn relabel_moves_or_merges() {
        let pool = test_pool().await;
        save_meeting_samples(&pool, "m1", &[sample("Speaker 1", [1.0, 0.0], 10.0), sample("Speaker 2", [0.0, 1.0], 30.0)]).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        relabel_in_meeting(&mut tx, "m1", "Speaker 1", "Dana").await.unwrap();
        relabel_in_meeting(&mut tx, "m1", "Speaker 2", "Dana").await.unwrap();
        tx.commit().await.unwrap();
        let v = list_voices(&pool).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Dana");
        let dana = &load_profiles(&pool, "none").await.unwrap()[0].1;
        assert!(dana[1] > dana[0], "merge weighted toward the 30 s sample");
    }

    #[tokio::test]
    async fn rename_and_forget_voice() {
        let pool = test_pool().await;
        save_meeting_samples(&pool, "m1", &[sample("Pria", [1.0, 0.0], 30.0)]).await.unwrap();
        save_meeting_samples(&pool, "m2", &[sample("Pria", [1.0, 0.0], 30.0)]).await.unwrap();
        rename_voice(&pool, "Pria", "Priya").await.unwrap();
        let v = list_voices(&pool).await.unwrap();
        assert_eq!((v[0].name.as_str(), v[0].meetings), ("Priya", 2));
        assert_eq!(forget_voice(&pool, "Priya").await.unwrap(), 2);
        assert!(list_voices(&pool).await.unwrap().is_empty());
    }
}
```

Run: `cargo test --no-default-features --features platform-default --lib voices` → fails until `pub mod voices;` added, then 4 passed. (If `tokio::test` needs the `macros` feature and it isn't enabled, use the pattern existing async tests in `database/repositories/summary.rs` use.)

- [ ] **Step 3: Rename updates samples** — replace the body of `api_rename_speaker` after validation:

```rust
    let state = app.try_state::<AppState>().ok_or("App state not available")?;
    let fail = |e: sqlx::Error| format!("Failed to rename speaker: {}", e);
    let mut tx = state.db_manager.pool().begin().await.map_err(fail)?;
    let n = sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
        .bind(to).bind(&meeting_id).bind(&from)
        .execute(&mut *tx).await.map_err(fail)?
        .rows_affected();
    // Teaches voice memory: the meeting's sample for `from` now belongs to `to`.
    crate::voices::relabel_in_meeting(&mut tx, &meeting_id, &from, to).await.map_err(fail)?;
    tx.commit().await.map_err(fail)?;
    Ok(n)
```

- [ ] **Step 4: Meeting delete** — in `meeting.rs`, before `// 3. Delete from transcripts`:

```rust
    sqlx::query("DELETE FROM voice_samples WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;
```

- [ ] **Step 5: Register** `voices::api_list_voices, voices::api_rename_voice, voices::api_forget_voice,` in `generate_handler!`. Run `cargo test ... --lib voices` → 4 passed; `cargo check --no-default-features --features platform-default` clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/migrations/20260926000000_add_voice_samples.sql frontend/src-tauri/src/voices.rs frontend/src-tauri/src/lib.rs frontend/src-tauri/src/diarization.rs frontend/src-tauri/src/database/repositories/meeting.rs
git commit -m "feat(voices): voice sample storage; renaming a speaker teaches its voice"
```

---

### Task 6: Name speakers from remembered voices during identification

**Files:**
- Modify: `frontend/src-tauri/src/voices.rs` (matching + labels + tests)
- Modify: `frontend/src-tauri/src/diarization.rs` (`resolve`/`diarize` return centroids; `identify` names + saves)

**Interfaces:**
- Consumes: Task 5 storage.
- Produces: `diarization::Diarization { track: Vec<Option<usize>>, centroids: Vec<Vec<f32>> }` returned by `diarize`; `diarization::speech_secs(&[Option<usize>], usize) -> Vec<f64>`; `voices::{VOICE_MATCH_THRESHOLD, MIN_SAMPLE_SECS, match_voices, labels}`. `identify` has a `names: Vec<Option<String>>` step before voice matching that Task 10 fills from hints.

- [ ] **Step 1: Failing tests in `voices.rs` tests module**

```rust
    #[test]
    fn match_is_one_to_one_best_first_and_thresholded() {
        let profiles = vec![("Priya".to_string(), normalize(vec![1.0, 0.0])), ("Dana".to_string(), normalize(vec![0.0, 1.0]))];
        // Speaker 0 is close to Priya, speaker 1 closer still, speaker 2 near nobody.
        let c = vec![normalize(vec![1.0, 0.3]), normalize(vec![1.0, 0.1]), normalize(vec![-1.0, -1.0])];
        let mut names = vec![None, None, None];
        match_voices(&c, &mut names, &profiles, 0.55);
        assert_eq!(names, vec![None, Some("Priya".into()), None]);
    }

    #[test]
    fn match_respects_names_already_given() {
        let profiles = vec![("Priya".to_string(), normalize(vec![1.0, 0.0]))];
        let c = vec![normalize(vec![1.0, 0.0]), normalize(vec![1.0, 0.05])];
        let mut names = vec![None, Some("Priya".to_string())];
        match_voices(&c, &mut names, &profiles, 0.55);
        assert_eq!(names, vec![None, Some("Priya".into())], "Priya already taken by a hint");
    }

    #[test]
    fn unnamed_speakers_are_numbered_consecutively() {
        assert_eq!(labels(&[None, Some("Priya".into()), None]), vec!["Speaker 1", "Priya", "Speaker 2"]);
    }
```

- [ ] **Step 2: Implement in `voices.rs`**

```rust
/// Cosine at or above which a meeting speaker is given a known voice's name.
/// ponytail: calibration knob; stricter than in-meeting clustering (0.40) because a wrong
/// name is worse than "Speaker 3". Re-check with diarization's `real_voice_matching`.
pub const VOICE_MATCH_THRESHOLD: f32 = 0.55;
/// Speakers with less speech than this aren't stored (short samples make unreliable voiceprints).
pub const MIN_SAMPLE_SECS: f64 = 20.0;

/// Fills unnamed speakers with known voices: greedy one-to-one on cosine (inputs are
/// L2-normalised, so dot product), best pairs first, skipping names already used.
pub fn match_voices(centroids: &[Vec<f32>], names: &mut [Option<String>], profiles: &[(String, Vec<f32>)], threshold: f32) {
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (s, c) in centroids.iter().enumerate() {
        if names[s].is_some() || c.is_empty() {
            continue;
        }
        for (p, (_, v)) in profiles.iter().enumerate() {
            let score: f32 = c.iter().zip(v).map(|(a, b)| a * b).sum();
            if score >= threshold {
                pairs.push((score, s, p));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, s, p) in pairs {
        let name = &profiles[p].0;
        if names[s].is_none() && !names.iter().any(|n| n.as_deref() == Some(name.as_str())) {
            names[s] = Some(name.clone());
        }
    }
}

/// Final label per speaker id: its name, else "Speaker N" numbered in id order.
pub fn labels(names: &[Option<String>]) -> Vec<String> {
    let mut n = 0;
    names.iter().map(|x| x.clone().unwrap_or_else(|| { n += 1; format!("Speaker {n}") })).collect()
}
```

Run `cargo test ... --lib voices` → 7 passed.

- [ ] **Step 3: Centroids out of diarization**

In `diarization.rs`:

```rust
/// Frame-level speaker track plus each speaker's centroid embedding (L2-normalised, indexed by id).
pub(crate) struct Diarization {
    pub track: Vec<Option<usize>>,
    pub centroids: Vec<Vec<f32>>,
}

/// Seconds of speech per speaker id in a frame track.
pub(crate) fn speech_secs(track: &[Option<usize>], k: usize) -> Vec<f64> {
    let mut secs = vec![0.0; k];
    for id in track.iter().flatten() {
        if *id < k {
            secs[*id] += FRAME_SECS;
        }
    }
    secs
}
```

Change `Extracted::resolve` to return `Diarization`: keep the body, but after `let ids = assign_speakers_scaled(...)` and `let k = ...` compute

```rust
        let mut centroids = vec![vec![0f32; self.embeddings.first().map_or(0, |e| e.len())]; k];
        for (e, &id) in self.embeddings.iter().zip(&ids) {
            centroids[id].iter_mut().zip(e).for_each(|(a, b)| *a += b);
        }
        let centroids = centroids.into_iter().map(crate::voices::normalize).collect();
```

and wrap the final frame iterator: `Diarization { track: (0..self.total_frames).map(...).collect(), centroids }`. `diarize` returns `Result<Diarization>`. Update callers: in `identify` the spawn_blocking closure returns `Result<Diarization>` and becomes `let Diarization { track, centroids } = ...;`; in tests `real_audio_file` and `real_meeting_folder` use `.track`.

Add a test next to the existing clustering tests:

```rust
    #[test]
    fn speech_secs_counts_frames_per_speaker() {
        let track = vec![Some(0), Some(0), None, Some(1)];
        let s = speech_secs(&track, 2);
        assert!((s[0] - 2.0 * FRAME_SECS).abs() < 1e-9 && (s[1] - FRAME_SECS).abs() < 1e-9);
    }
```

- [ ] **Step 4: Name and save in `identify`**

Right after the diarization result, before `// Turns per line`:

```rust
    let k = centroids.len();
    let secs = speech_secs(&track, k);
    // Names from the meeting app's "who is speaking" hints (Part 4) go here, then known voices.
    let mut names: Vec<Option<String>> = vec![None; k];
    match crate::voices::load_profiles(&pool, meeting_id).await {
        Ok(profiles) => crate::voices::match_voices(&centroids, &mut names, &profiles, crate::voices::VOICE_MATCH_THRESHOLD),
        Err(e) => warn!("Voice memory unavailable for {}: {}", meeting_id, e),
    }
    let labels = crate::voices::labels(&names);
```

Replace `let speaker = |id: usize| format!("Speaker {}", id + 1);` with:

```rust
    let speaker = |id: usize| labels.get(id).cloned().unwrap_or_else(|| format!("Speaker {}", id + 1));
```

After `tx.commit().await?;`:

```rust
    let samples: Vec<crate::voices::Sample> = (0..k)
        .filter(|&id| secs[id] >= crate::voices::MIN_SAMPLE_SECS && !centroids[id].is_empty())
        .map(|id| crate::voices::Sample { label: labels[id].clone(), embedding: centroids[id].clone(), speech_secs: secs[id] })
        .collect();
    if let Err(e) = crate::voices::save_meeting_samples(&pool, meeting_id, &samples).await {
        warn!("Couldn't save voice samples for {}: {}", meeting_id, e);
    }
```

- [ ] **Step 5: Run** `cargo test --no-default-features --features platform-default --lib diarization` and `--lib voices` → all pass (ignored tests skipped); `cargo check` clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/voices.rs frontend/src-tauri/src/diarization.rs
git commit -m "feat(voices): name speakers from remembered voices and save each meeting's voiceprints"
```

---

### Task 7: Settings → Speakers

**Files:**
- Create: `frontend/src/components/SpeakerSettings.tsx`
- Modify: `frontend/src/app/settings/page.tsx`

**Interfaces:**
- Consumes: commands `api_list_voices` → `{ name: string; meetings: number; last_heard: string }[]`, `api_rename_voice({ from, to })`, `api_forget_voice({ name })`.

- [ ] **Step 1: Component**

```tsx
'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';

interface Voice {
  name: string;
  meetings: number;
  last_heard: string;
}

export function SpeakerSettings() {
  const [voices, setVoices] = useState<Voice[] | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState('');

  const load = useCallback(async () => {
    try {
      setVoices(await invoke<Voice[]>('api_list_voices'));
    } catch (e) {
      toast.error('Failed to load voices', { description: String(e) });
      setVoices([]);
    }
  }, []);
  useEffect(() => { load(); }, [load]);

  const rename = async (from: string) => {
    const to = draft.trim();
    setEditing(null);
    if (!to || to === from) return;
    try {
      await invoke('api_rename_voice', { from, to });
      await load();
    } catch (e) {
      toast.error('Rename failed', { description: String(e) });
    }
  };

  const forget = async (name: string) => {
    if (!window.confirm(`Forget ${name}'s voice? Past transcripts keep the name.`)) return;
    try {
      await invoke('api_forget_voice', { name });
      await load();
    } catch (e) {
      toast.error('Forget failed', { description: String(e) });
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-semibold mb-2">Speakers</h3>
        <p className="text-sm text-muted-foreground">
          Rename a speaker in any meeting and Noetis recognises their voice next time.
          Voiceprints are stored only on this computer.
        </p>
      </div>
      {voices === null ? (
        <div className="h-8 animate-pulse rounded bg-accent" />
      ) : voices.length === 0 ? (
        <p className="text-sm text-muted-foreground">No known voices yet.</p>
      ) : (
        <ul className="divide-y rounded-lg border">
          {voices.map((v) => (
            <li key={v.name} className="flex items-center gap-3 p-3">
              <div className="min-w-0 flex-1">
                {editing === v.name ? (
                  <Input
                    autoFocus
                    value={draft}
                    maxLength={64}
                    aria-label={`New name for ${v.name}`}
                    onChange={(e) => setDraft(e.target.value)}
                    onBlur={() => rename(v.name)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') rename(v.name);
                      if (e.key === 'Escape') setEditing(null);
                    }}
                  />
                ) : (
                  <>
                    <div className="truncate font-medium">{v.name}</div>
                    <div className="text-xs text-muted-foreground">
                      {v.meetings} {v.meetings === 1 ? 'meeting' : 'meetings'} · last heard{' '}
                      {new Date(v.last_heard).toLocaleDateString()}
                    </div>
                  </>
                )}
              </div>
              <Button variant="ghost" size="sm" onClick={() => { setEditing(v.name); setDraft(v.name); }}>
                Rename
              </Button>
              <Button variant="ghost" size="sm" className="text-destructive" onClick={() => forget(v.name)}>
                Forget
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Tab** — in `settings/page.tsx` import `Users` from `lucide-react` and `SpeakerSettings`; add `{ value: 'speakers', label: 'Speakers', icon: Users },` after the `recording` entry in `TABS`; render `{activeTab === 'speakers' && <SpeakerSettings />}` after the recording line.

- [ ] **Step 3: Checks** — `tsc --noEmit`, `check-colors.mjs`, `check-contrast.mjs` clean.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/SpeakerSettings.tsx frontend/src/app/settings/page.tsx
git commit -m "feat(voices): Settings > Speakers to rename or forget known voices"
```

---

### Task 8: Voice-matching calibration test

**Files:**
- Modify: `frontend/src-tauri/src/diarization.rs` (tests module)

- [ ] **Step 1: Add the ignored test**

```rust
    /// Voice memory across recordings, label-free: diarize the whole file as reference, enrol
    /// voices from the first half, match the second half. A match is right when both halves'
    /// speakers map to the same reference speaker. Needs DIARIZATION_TEST_DIR, DIARIZATION_AUDIO.
    #[test]
    #[ignore]
    fn real_voice_matching() {
        use crate::voices::{match_voices, MIN_SAMPLE_SECS};
        let dir = PathBuf::from(std::env::var("DIARIZATION_TEST_DIR").unwrap());
        let file = PathBuf::from(std::env::var("DIARIZATION_AUDIO").unwrap());
        let audio = crate::audio::decoder::decode_audio_file(&file).unwrap().to_whisper_format();
        let mut seg = Segmenter::load(&dir.join(SEGMENTATION_MODEL.file)).unwrap();
        let mut emb = Embedder::load(&dir.join(EMBEDDING_MODEL.file)).unwrap();
        let params = test_params();
        let full = diarize(&mut seg, &mut emb, &audio, &params, |_| {}).unwrap();
        let mid = audio.len() / 2 / FRAME_STEP_SAMPLES * FRAME_STEP_SAMPLES;
        let a = diarize(&mut seg, &mut emb, &audio[..mid], &params, |_| {}).unwrap();
        let b = diarize(&mut seg, &mut emb, &audio[mid..], &params, |_| {}).unwrap();

        // Reference speaker most often under each half-speaker's frames.
        let majority = |d: &Diarization, offset: usize| -> Vec<Option<usize>> {
            (0..d.centroids.len())
                .map(|id| {
                    let mut counts = std::collections::HashMap::new();
                    for (f, s) in d.track.iter().enumerate() {
                        if *s == Some(id) {
                            if let Some(Some(r)) = full.track.get(offset + f) {
                                *counts.entry(*r).or_insert(0usize) += 1;
                            }
                        }
                    }
                    counts.into_iter().max_by_key(|(_, n)| *n).map(|(r, _)| r)
                })
                .collect()
        };
        let (ra, rb) = (majority(&a, 0), majority(&b, mid / FRAME_STEP_SAMPLES));
        let secs_a = speech_secs(&a.track, a.centroids.len());
        let profiles: Vec<(String, Vec<f32>)> = a.centroids.iter().enumerate()
            .filter(|(i, c)| secs_a[*i] >= MIN_SAMPLE_SECS && !c.is_empty())
            .map(|(i, c)| (format!("ref{}", ra[i].map_or(99, |r| r)), c.clone()))
            .collect();
        println!("reference speakers: {}, enrolled: {}", full.centroids.len(), profiles.len());
        for t in [0.40f32, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70] {
            let mut names = vec![None; b.centroids.len()];
            match_voices(&b.centroids, &mut names, &profiles, t);
            let (mut right, mut wrong, mut unnamed) = (0, 0, 0);
            for (i, n) in names.iter().enumerate() {
                match n {
                    Some(n) if rb[i].map(|r| format!("ref{r}")).as_deref() == Some(n.as_str()) => right += 1,
                    Some(_) => wrong += 1,
                    None => unnamed += 1,
                }
            }
            println!("threshold {t:.2}: right {right}, wrong {wrong}, unnamed {unnamed}");
        }
    }
```

- [ ] **Step 2: Compile** — `cargo test --no-default-features --features platform-default --lib diarization` → passes (test ignored).

- [ ] **Step 3: Run if a multi-speaker recording exists** (models live in `%APPDATA%\com.noetis.app\models\diarization`; recordings in `%USERPROFILE%\Music\noetis-recordings\*\audio.mp4`):

```bash
DIARIZATION_TEST_DIR="$APPDATA/com.noetis.app/models/diarization" DIARIZATION_AUDIO="<recording>" cargo test --release --no-default-features --features platform-default --lib real_voice_matching -- --ignored --nocapture
```

Record the table in the task report. If a threshold other than 0.55 gives zero wrong with more right, update `VOICE_MATCH_THRESHOLD` and its comment with the evidence. If no multi-speaker recording exists, report that and keep 0.55.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/diarization.rs frontend/src-tauri/src/voices.rs
git commit -m "test(voices): label-free cross-recording voice matching calibration"
```

---

### Task 9: Part 4 Step 0: UI Automation dump tool

**Files:**
- Create: `frontend/src-tauri/examples/uia_dump.rs`
- Modify: `frontend/src-tauri/Cargo.toml` (add `"Win32_UI_Accessibility", "Win32_System_Com", "Win32_System_Ole", "Win32_System_Variant"` to the `windows` features as the compiler requires)

- [ ] **Step 1: Tool**

```rust
//! Part 4 spike: dump what Zoom / Teams expose through UI Automation, to see whether "who is
//! speaking" can be read. Run during a call, output to a file:
//!   cargo run --example uia_dump --no-default-features --features platform-default -- zoom teams > dump.txt
//! Lines marked `*` mention speaking/talking/muted.

#[cfg(not(windows))]
fn main() {
    eprintln!("uia_dump is Windows only");
}

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    let filters: Vec<String> = {
        let args: Vec<String> = std::env::args().skip(1).map(|a| a.to_lowercase()).collect();
        if args.is_empty() { vec!["zoom".into(), "teams".into()] } else { args }
    };
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let walker = uia.RawViewWalker()?;
        let root = uia.GetRootElement()?;
        let mut win = walker.GetFirstChildElement(&root).ok();
        while let Some(w) = win {
            let name = w.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            let class = w.CurrentClassName().map(|b| b.to_string()).unwrap_or_default();
            let hay = format!("{name} {class}").to_lowercase();
            if filters.iter().any(|f| hay.contains(f)) {
                println!("=== window: {name:?} class={class:?}");
                dump(&walker, &w, 1);
            }
            win = walker.GetNextSiblingElement(&w).ok();
        }
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn dump(walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker, el: &windows::Win32::UI::Accessibility::IUIAutomationElement, depth: usize) {
    if depth > 60 {
        return;
    }
    let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
    let id = el.CurrentAutomationId().map(|b| b.to_string()).unwrap_or_default();
    let class = el.CurrentClassName().map(|b| b.to_string()).unwrap_or_default();
    let ctype = el.CurrentControlType().map(|t| t.0).unwrap_or(0);
    let offscreen = el.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(false);
    let lower = name.to_lowercase();
    let mark = if ["speaking", "talking", "muted", "unmuted"].iter().any(|k| lower.contains(k)) { '*' } else { ' ' };
    println!("{mark}{:indent$}[{ctype}] name={name:?} id={id:?} class={class:?}{}", "", if offscreen { " offscreen" } else { "" }, indent = depth * 2);
    let mut child = walker.GetFirstChildElement(el).ok();
    while let Some(c) = child {
        dump(walker, &c, depth + 1);
        child = walker.GetNextSiblingElement(&c).ok();
    }
}
```

- [ ] **Step 2: Build** — `cargo build --example uia_dump --no-default-features --features platform-default` succeeds; running it with no call open prints nothing and exits 0.

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/examples/uia_dump.rs frontend/src-tauri/Cargo.toml Cargo.lock
git commit -m "chore(hints): UI Automation dump tool for the Zoom/Teams speaker-hint spike"
```

- [ ] **Step 4: USER GATE** — ask the user to run the tool during a real Zoom call and a real Teams call (participants panel open, someone talking) and share the output. The per-app readers and the in-call watcher are planned in a follow-up plan written from those dumps. If neither dump contains a speaking indicator, Part 4 stops after Task 10.

---

### Task 10: Name speakers from `speaker-hints.json`

**Files:**
- Create: `frontend/src-tauri/src/speaker_hints.rs`
- Modify: `frontend/src-tauri/src/lib.rs` (`pub mod speaker_hints;`)
- Modify: `frontend/src-tauri/src/diarization.rs` (`identify` hint step)

**Interfaces:**
- Consumes: `Diarization.track`, `FRAME_SECS` (Task 6 naming step).
- Produces: `speaker_hints::{Hint { name, start, end }, HINTS_FILE, load(&Path) -> Vec<Hint>, names_from_hints(&[Option<usize>], usize, f64, &[Hint]) -> Vec<Option<String>>}`. The follow-up watcher writes `HINTS_FILE` as a JSON array of `Hint`.

- [ ] **Step 1: Module with failing tests**

```rust
//! Speaker names from the meeting app's own "who is speaking" indicator (Zoom / Teams),
//! recorded during the call into the meeting folder as `speaker-hints.json`.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const HINTS_FILE: &str = "speaker-hints.json";
/// A name must cover this share of a speaker's speech...
const MIN_SHARE: f64 = 0.6;
/// ...and at least this much of it.
const MIN_SECS: f64 = 10.0;

/// `name` was shown speaking from `start` to `end` seconds after the recording started.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hint {
    pub name: String,
    pub start: f64,
    pub end: f64,
}

/// Hints for a meeting folder; missing or unreadable file = no hints.
pub fn load(folder: &Path) -> Vec<Hint> {
    let path = folder.join(HINTS_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        log::warn!("Ignoring unreadable {}: {}", path.display(), e);
        Vec::new()
    })
}

/// Name per speaker id: greedy one-to-one by overlap, a name needing MIN_SHARE of the
/// speaker's speech and MIN_SECS.
pub fn names_from_hints(track: &[Option<usize>], k: usize, frame_secs: f64, hints: &[Hint]) -> Vec<Option<String>> {
    let mut names: Vec<&str> = hints.iter().map(|h| h.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    let mut overlap = vec![vec![0.0f64; names.len()]; k];
    let mut speech = vec![0.0f64; k];
    for (f, s) in track.iter().enumerate() {
        let Some(s) = *s else { continue };
        if s >= k {
            continue;
        }
        speech[s] += frame_secs;
        let t = (f as f64 + 0.5) * frame_secs;
        for (n, name) in names.iter().enumerate() {
            if hints.iter().any(|h| h.name == *name && h.start <= t && t < h.end) {
                overlap[s][n] += frame_secs;
            }
        }
    }
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for s in 0..k {
        for n in 0..names.len() {
            let o = overlap[s][n];
            if o >= MIN_SECS && o >= MIN_SHARE * speech[s] {
                pairs.push((o, s, n));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out: Vec<Option<String>> = vec![None; k];
    let mut used = vec![false; names.len()];
    for (_, s, n) in pairs {
        if out[s].is_none() && !used[n] {
            out[s] = Some(names[n].to_string());
            used[n] = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(name: &str, start: f64, end: f64) -> Hint {
        Hint { name: name.into(), start, end }
    }

    /// 1 frame = 1 s for readability.
    fn track(spans: &[(usize, usize, usize)]) -> Vec<Option<usize>> {
        let len = spans.iter().map(|s| s.2).max().unwrap_or(0);
        let mut t = vec![None; len];
        for &(id, a, b) in spans {
            t[a..b].iter_mut().for_each(|x| *x = Some(id));
        }
        t
    }

    #[test]
    fn names_speakers_whose_speech_the_hint_covers() {
        let t = track(&[(0, 0, 20), (1, 20, 40)]);
        let h = vec![hint("Priya", 0.0, 20.0), hint("Marcus", 20.0, 40.0)];
        assert_eq!(names_from_hints(&t, 2, 1.0, &h), vec![Some("Priya".into()), Some("Marcus".into())]);
    }

    #[test]
    fn needs_sixty_percent_and_ten_seconds() {
        let t = track(&[(0, 0, 20), (1, 20, 28)]);
        // Priya covers 11/20 of speaker 0 (55 %); Marcus covers all 8 s of speaker 1 (< 10 s).
        let h = vec![hint("Priya", 0.0, 11.0), hint("Marcus", 20.0, 28.0)];
        assert_eq!(names_from_hints(&t, 2, 1.0, &h), vec![None, None]);
        let h = vec![hint("Priya", 0.0, 12.0)];
        assert_eq!(names_from_hints(&t, 2, 1.0, &h)[0].as_deref(), Some("Priya"), "12/20 = 60 %");
    }

    #[test]
    fn one_name_per_speaker_largest_overlap_first() {
        // Overlapping talk: Priya's interval covers both speakers; she goes to the larger overlap.
        let t = track(&[(0, 0, 15), (1, 15, 40)]);
        let h = vec![hint("Priya", 0.0, 40.0)];
        assert_eq!(names_from_hints(&t, 2, 1.0, &h), vec![None, Some("Priya".into())]);
        assert_eq!(names_from_hints(&t, 2, 1.0, &[]), vec![None, None]);
    }

    #[test]
    fn load_reads_the_meeting_folder() {
        let dir = std::env::temp_dir().join(format!("noetis-hints-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load(&dir).is_empty());
        std::fs::write(dir.join(HINTS_FILE), r#"[{"name":"Priya","start":1.5,"end":4.0}]"#).unwrap();
        assert_eq!(load(&dir), vec![hint("Priya", 1.5, 4.0)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
```

Run: `cargo test --no-default-features --features platform-default --lib speaker_hints` → 4 passed.

- [ ] **Step 2: Use in `identify`** — replace `let mut names: Vec<Option<String>> = vec![None; k];` with:

```rust
    let hints = crate::speaker_hints::load(folder);
    let mut names = crate::speaker_hints::names_from_hints(&track, k, FRAME_SECS, &hints);
```

(Keep the comment line above it; update it to "Meeting-app hints first, then known voices.")

- [ ] **Step 3: Run** `--lib diarization`, `--lib voices`, `--lib speaker_hints` → pass; `cargo check` clean.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/speaker_hints.rs frontend/src-tauri/src/lib.rs frontend/src-tauri/src/diarization.rs
git commit -m "feat(hints): name speakers from recorded meeting-app speaking intervals"
```

---

## After all tasks

- Full check: `cargo test --no-default-features --features platform-default --lib` and frontend `tsc`, colour, contrast, `node_modules/.bin/next build`.
- Manual smoke (user, Windows), from the spec: join a test call → prompt → Record → leave → auto-stop, speakers, summary with window hidden; rename a speaker, second call → name applied.
- Follow-up plan (after Task 9's user gate): Zoom/Teams UIA readers + in-call watcher writing `speaker-hints.json`.
