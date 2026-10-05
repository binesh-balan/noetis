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
