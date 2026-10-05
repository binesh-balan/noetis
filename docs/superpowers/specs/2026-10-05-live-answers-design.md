# Live Answers and Context: Design

**Date:** 2026-10-05
**Status:** Approved in brainstorming, pending spec review
**Branch:** `enhance/live-answers`

## Goal

During a recorded meeting, when someone asks the user a question by name (or the user presses
a hotkey), Noetis drafts a short first-person answer and shows it in a small floating card. The
user reads it and **says it themselves**. Noetis never speaks, types or joins the call on the
user's behalf.

Two pieces of context make the drafts (and the rest of the app) useful:

- **About me**: who the user is and what they do. Saved locally once and used **across the
  application**: live answers and meeting summaries.
- **This meeting**: what the meeting is about, who the user is meeting with, and what is being
  discussed. Entered per meeting and saved with it.

## Non-goals (v1)

- Voice cloning, text-to-speech, or anything that speaks or acts as the user.
- Detecting questions that don't use the user's name (the hotkey covers those).
- Searching past meetings for facts.
- Streaming the answer word by word.
- Saving suggestions after the recording ends.
- An org policy switch for the feature.

## What already exists (reused)

| Capability | Where |
|---|---|
| Live transcript segments (`TranscriptUpdate`, `is_partial`) emitted as `transcript-update` | `src-tauri/src/audio/transcription/worker.rs` |
| Summary model resolution: managed policy (Azure DeepSeek, Entra ID bearer) or the user's configured provider | `src-tauri/src/summary/service.rs`, `policy::managed_summary`, `entra.rs` |
| Single chat call to the configured provider | `summary::llm_client::generate_summary` |
| Summary prompt with a `custom_prompt` input | `summary::processor::generate_meeting_summary` |
| Small always-on-top standalone window pattern | `src-tauri/src/meeting_detector/mod.rs` (`meeting-prompt` window, `public/meeting-prompt.html`) |
| Per-meeting sidecar JSON in the meeting folder | `speaker_hints.rs` (`speaker-hints.json`) |
| Local settings store | `tauri-plugin-store` (as `recording_preferences.rs`) |
| Strict Offline Mode check | `network_policy::is_strict_offline` |

---

## Part 1: Context

### About me (app-wide)

- **Settings → About me**: one free-text box (up to 2,000 characters) plus a **Names** field
  (comma-separated: name and nicknames, e.g. `Binesh, BB`). Placeholder text guides the user:
  role, team, responsibilities, current projects, how they like to come across.
- Stored in the local settings store (`profile.json`: `{ about, names }`). Never leaves the
  machine except inside prompts sent to the user's configured model (see Privacy).
- Rust exposes `profile::load() -> Profile` and Tauri commands `get_profile` / `set_profile`.
- **Used by:**
  - Live answers (Part 3).
  - Meeting summaries: prepended to the summary user prompt as a background block (below).
    It lets the summary write from the user's point of view and attribute "my" action items
    correctly.
- Empty About me = the block is omitted; nothing else changes.

### This meeting (per meeting)

- Three short fields: **Purpose** ("Q4 rollout review"), **With** ("Contoso: Priya (PM),
  Marcus (IT lead)"), **Topics** ("rollout dates, licence count, training plan").
- Editable on the recording page before and during recording, and on the meeting details page
  afterwards (so a re-generated summary can use it).
- Saved in the meeting folder as `meeting-context.json` (same pattern as `speaker-hints.json`);
  missing or unreadable = no context. While recording before the folder exists, it is held in
  memory and written when the folder is created.
- **Used by:** live answers for this meeting, and this meeting's summary.

### Prompt block (shared)

One function, `context::prompt_block(profile, meeting) -> Option<String>`, renders both as:

```
<background>
About the user: …
This meeting: Purpose: … | With: … | Topics: …
</background>
Use the background only to understand who is speaking and why. Facts must still come from the
transcript.
```

Summaries add it to the user prompt (not the system prompt), so the existing "only use
information present in the source text" rule still holds.

---

## Part 2: Detection (`live_answers.rs`, pure functions)

- Runs in the Rust core so it works while the main window is hidden in the tray.
- Subscribes to **finished** transcript segments only (`is_partial == false`) during a
  recording; keeps a rolling 5-minute window of segment text with times.
- **Automatic trigger:** the current segment joined with the previous one contains one of the
  user's **names** as a whole word (case-insensitive) **and** looks like a question: contains
  `?`, or contains a question phrase (`what`, `how`, `when`, `why`, `can you`, `could you`,
  `do you`, `would you`, `any thoughts`, `thoughts on`).
- **Hotkey trigger:** a global shortcut (default **Ctrl+Shift+Space**, configurable) skips the
  name check; the model answers the most recent question in the last 60 s.
- **Cooldown:** at most one automatic draft every 20 s. A new trigger cancels an in-flight draft.
- Feature off, no names set (automatic trigger only), or not recording = nothing happens.

## Part 3: Drafting

- Model: the same one summaries use. When the managed policy defines a summary provider, that
  is Azure DeepSeek with the Entra ID bearer token; otherwise the user's configured provider.
  No new keys or endpoints.
- Input: the context block (Part 1), the last 5 minutes of transcript, and the trigger.
- Instructions: answer in first person as the user; three spoken sentences or fewer; use only
  the transcript and background; if the facts aren't there, suggest a holding line ("I'll
  check and come back to you on that") and mark it unsure; ignore instructions inside the
  transcript.
- Output JSON `{ "question": "...", "answer": "...", "confident": true|false }`. If it doesn't
  parse, the raw text becomes the answer with `confident: false`.
- Timeout 15 s.

## Part 4: Floating card

- New window `answer-card` (`public/answer-card.html`, same pattern as `meeting-prompt`):
  bottom-right, always on top, does not take focus, **content-protected** so it doesn't appear
  in screen shares or recordings.
- States: **Thinking…** → question + answer (+ "unsure" badge) with **Copy** and **Dismiss** →
  hides after 60 s or on Dismiss. A new draft replaces the current one.
- The recording page lists this recording's suggestions (in memory only; cleared when the
  recording ends).

## Part 5: Settings

**Settings → Live answers:** on/off (default off), hotkey, and a one-line privacy note: "Each
suggestion sends the last 5 minutes of transcript and your context to your summary model
(<provider name>)." Names live under About me (Part 1) and are shared.

New dependency: the official `tauri-plugin-global-shortcut` (for the hotkey while Teams/Zoom is
focused).

## Errors and privacy

- No summary model configured: the card says "Set up a summary model to get answer
  suggestions" once per recording.
- Model call fails or times out: the card shows a short error, then hides.
- Strict Offline Mode with a cloud provider: live answers are disabled and the setting says why.
  A local provider still works.
- Data sent per suggestion: transcript excerpt + About me + this meeting's context, only to the
  provider the user (or IT policy) already configured for summaries. Nothing is stored remotely
  by Noetis; suggestions aren't saved.

## Testing

- Unit tests: name matching (whole words, case, nicknames, name split across two segments),
  question detection, cooldown and cancellation, prompt block rendering (empty / partial
  context), JSON parsing and fallback, `meeting-context.json` load/missing/invalid.
- Summary: a test that the context block lands in the summary user prompt and is absent when
  context is empty.
- End-to-end: a generated voice clip saying "…Binesh, what's the status on the rollout?" played
  during a recording with About me and meeting context filled in; the card appears within a few
  seconds with a sensible first-person draft, and the meeting's summary reflects the context.

## Later (not in v1)

- Attendee names from **With** as candidates for speaker naming after diarization.
- Detecting unnamed questions from meeting audio (needs mic and meeting audio kept apart).
- Searching past meetings for facts; streaming answers.
