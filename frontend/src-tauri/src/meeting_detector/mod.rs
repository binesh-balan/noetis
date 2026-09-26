//! Meeting detection (Windows): notices a call app holding the microphone and offers to
//! record it. `signal` reads who holds the mic; everything here is pure and unit-tested.

#[cfg(target_os = "windows")]
mod signal;

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
