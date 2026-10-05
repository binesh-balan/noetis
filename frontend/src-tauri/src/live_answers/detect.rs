//! Pure trigger logic: has someone just asked the user a question by name?

use std::time::{Duration, Instant};

/// Transcript kept for drafting.
pub const WINDOW_SECS: f64 = 300.0;
/// Minimum gap between automatic drafts.
pub const COOLDOWN: Duration = Duration::from_secs(20);
const QUESTION_PHRASES: [&str; 10] =
    ["what", "how", "when", "why", "can you", "could you", "do you", "would you", "any thoughts", "thoughts on"];

/// A finished transcript segment (seconds from the start of the recording).
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Any of `names` as whole words (case-insensitive); multi-word names must appear in order.
pub fn mentions_name(text: &str, names: &[String]) -> bool {
    let text = words(text);
    names.iter().any(|name| {
        let n = words(name);
        !n.is_empty() && text.windows(n.len()).any(|w| w == n.as_slice())
    })
}

pub fn looks_like_question(text: &str) -> bool {
    if text.contains('?') {
        return true;
    }
    let padded = format!(" {} ", words(text).join(" "));
    QUESTION_PHRASES.iter().any(|p| padded.contains(&format!(" {p} ")))
}

#[derive(Default)]
pub struct Detector {
    segments: Vec<Segment>,
    last_auto: Option<Instant>,
}

impl Detector {
    /// Adds a finished segment. True when it, joined with the previous one, names the user and
    /// asks a question, outside the cooldown. Empty `names` never triggers.
    pub fn push(&mut self, seg: Segment, names: &[String], now: Instant) -> bool {
        let joined = match self.segments.last() {
            Some(prev) => format!("{} {}", prev.text, seg.text),
            None => seg.text.clone(),
        };
        let cutoff = seg.end - WINDOW_SECS;
        self.segments.retain(|s| s.end >= cutoff);
        self.segments.push(seg);
        if names.is_empty() || !mentions_name(&joined, names) || !looks_like_question(&joined) {
            return false;
        }
        if self.last_auto.is_some_and(|t| now.duration_since(t) < COOLDOWN) {
            return false;
        }
        self.last_auto = Some(now);
        true
    }

    /// The window as "[mm:ss] text" lines, oldest first.
    pub fn transcript(&self) -> String {
        self.segments
            .iter()
            .map(|s| {
                let t = s.start.max(0.0) as u64;
                format!("[{:02}:{:02}] {}", t / 60, t % 60, s.text.trim())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }
    fn seg(start: f64, text: &str) -> Segment {
        Segment { start, end: start + 2.0, text: text.into() }
    }

    #[test]
    fn names_match_whole_words_any_case() {
        let n = names(&["Alex", "AJ", "Alex Morgan"]);
        assert!(mentions_name("alex, what's the status?", &n));
        assert!(mentions_name("Over to you, AJ.", &n));
        assert!(!mentions_name("Alexander will join later", &n));
        assert!(!mentions_name("ajax deploy failed", &n));
        assert!(mentions_name("Thanks Alex Morgan", &names(&["Alex Morgan"])));
        assert!(!mentions_name("Morgan and Alex", &names(&["Alex Morgan"])));
        assert!(!mentions_name("anything", &names(&[" "])));
    }

    #[test]
    fn questions_by_mark_or_phrase() {
        assert!(looks_like_question("Is it ready?"));
        assert!(looks_like_question("Alex could you walk us through it"));
        assert!(looks_like_question("any thoughts on the budget"));
        assert!(!looks_like_question("Alex, the deck is ready."));
        assert!(!looks_like_question("somewhat late"), "phrase must be whole words");
    }

    #[test]
    fn triggers_on_name_and_question_even_across_segments() {
        let n = names(&["Alex"]);
        let t0 = Instant::now();
        let mut d = Detector::default();
        assert!(!d.push(seg(0.0, "Let's move on."), &n, t0));
        assert!(!d.push(seg(2.0, "Alex,"), &n, t0));
        assert!(d.push(seg(4.0, "what's the status of the rollout?"), &n, t0));
        let mut d = Detector::default();
        assert!(!d.push(seg(0.0, "What's the status?"), &[], t0), "no names, no trigger");
    }

    #[test]
    fn cooldown_spaces_out_automatic_drafts() {
        let n = names(&["Alex"]);
        let t0 = Instant::now();
        let mut d = Detector::default();
        assert!(d.push(seg(0.0, "Alex, any update?"), &n, t0));
        assert!(!d.push(seg(5.0, "Alex, and the dates?"), &n, t0 + Duration::from_secs(5)));
        assert!(d.push(seg(30.0, "Alex, and the budget?"), &n, t0 + Duration::from_secs(21)));
    }

    #[test]
    fn window_keeps_five_minutes_as_timed_lines() {
        let mut d = Detector::default();
        let t0 = Instant::now();
        d.push(seg(0.0, "first"), &[], t0);
        d.push(seg(65.0, " second "), &[], t0);
        assert_eq!(d.transcript(), "[00:00] first\n[01:05] second");
        d.push(seg(400.0, "late"), &[], t0);
        assert_eq!(d.transcript(), "[06:40] late");
    }
}
