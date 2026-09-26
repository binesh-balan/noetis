# Auto Meeting Capture: Design

**Date:** 2026-09-26
**Status:** Approved in brainstorming, pending spec review
**Branch:** `enhance/auto-meeting-capture`

## Goal

Noetis notices when the user is in a Zoom / Teams / Meet / Slack / Webex / Discord call, asks
whether to record, records it, and afterwards produces a transcript with **real speaker names**
plus a summary with action items, without the user pressing Record or renaming speakers.

Everything stays on the user's PC. No meeting bot, no server, no cloud calls added.

## What already exists (reused, not rebuilt)

| Capability | Where |
|---|---|
| Live transcription during recording | `src-tauri/src/audio/recording_commands.rs` (`start_transcription_task`) |
| Post-meeting diarization ("Speaker N") | `src-tauri/src/diarization.rs` (`identify`, auto-started by `src/hooks/useRecordingStop.ts` via `src/lib/speakers.ts`) |
| Rename speaker across a meeting | `diarization.rs` `api_rename_speaker` |
| Auto summary after recording (waits for diarization) | `src/app/meeting-details/page.tsx` `setupAutoGeneration`, gated on `isAutoSummary` + configured model |
| Summary template with Summary / Key Decisions / Action Items / Discussion Highlights | `src-tauri/src/summary/templates/defaults.rs` (`standard_meeting`) |
| Tray start/stop recording flow | `src-tauri/src/tray.rs` (start: `sessionStorage.autoStartRecording` + navigate `/`; stop: `stop_recording` + `recording-stop-complete`) |

## Non-goals

- A bot that joins calls as a participant (needs a server, breaks local-only). Possible later
  as an opt-in "Send notetaker" add-on that feeds the same pipeline; out of scope here.
- Meeting detection on macOS / Linux (macOS can't attribute mic use to a process cheaply).
  The detector compiles to a no-op there and the setting is hidden.
- Calendar integration.
- Fetching official Zoom/Teams cloud transcripts.

---

## Part 1: Meeting detection and recording (Windows)

### Detector

New module `src-tauri/src/meeting_detector.rs`, one background task started in `lib.rs` setup.

- **Signal:** every 2 s, read the Windows capability-access store:
  `HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone`
  - `NonPackaged\<exe path with # separators>` subkeys (Win32 apps: Zoom, classic Teams, browsers, Slack, Discord, Webex)
  - packaged-app subkeys (new Teams `MSTeams_*`)
  - An entry is **holding the mic** when `LastUsedTimeStop == 0` and `LastUsedTimeStart != 0`.
- **Meeting apps** (matched on exe file name / package prefix, case-insensitive):
  `zoom.exe`, `teams.exe`, `ms-teams.exe` / `MSTeams_*`, `slack.exe`, `webexmta.exe` / `CiscoCollabHost.exe`, `discord.exe`.
- **Browsers** (`chrome.exe`, `msedge.exe`, `firefox.exe`, `brave.exe`) count only when a
  top-level window of that process has a meeting-like title: contains `Meet -`, `meet.google.com`,
  `Microsoft Teams`, `Zoom`, `Webex`, or `Whereby`. (Window titles via `EnumWindows` + `GetWindowTextW` + `GetWindowThreadProcessId`.)
- Noetis' own exe is always excluded.
- **State machine** (pure, unit-tested, fed `(now, Vec<MeetingApp>)`):
  - `Idle` → `InMeeting{app}` once any meeting app has held the mic continuously for **5 s**.
  - `InMeeting` → `Idle` once **no** meeting app has held the mic for **15 s** (grace for mute/unmute).
  - Multiple meeting apps at once = one meeting (the first detected app names it).
  - Emits `MeetingStarted{app}` / `MeetingEnded` transitions only.
- If the registry key can't be opened, log once at `warn` and stop the detector for the session.
- Constants (`POLL`, `START_AFTER`, `END_AFTER`) are named `const`s with `ponytail:` calibration comments.

### Prompt ("Ask first", the default)

On `MeetingStarted`, if no recording is active and the mode is not `Off`:

- Mode `Ask`: open a small frameless, always-on-top, non-focus-stealing Tauri window
  (label `meeting-prompt`, ~360x120, bottom-right of the primary monitor) rendering route
  `/meeting-prompt`: "🎙 {App} meeting detected", buttons **Record** / **Not now**.
  Auto-dismisses after **60 s**. If auto-summary is off or no summary model is configured, it also
  shows one muted line: "Summary won't auto-generate: set up a model".
- Mode `Auto`: skip the prompt and start directly (a Windows toast says "Recording {App} meeting").
- "Not now" / timeout suppresses prompts until the next `MeetingEnded`.
- If recording fails to start, the prompt window shows the error text instead of closing.

### Start / stop

- **Record** reuses the tray start path: set `sessionStorage.autoStartRecording = 'true'` and
  `sessionStorage.autoStartMeetingName = "{App} meeting, {Mon D, HH:mm}"` on the `main` webview,
  navigate it to `/`. `useRecordingStart` reads the name when present (falls back to its current default).
- The detector remembers that **it** started this recording (`AtomicBool detector_owned`).
- On `MeetingEnded`, if `detector_owned` and still recording: stop via the same function the tray
  stop uses (extract the tray stop body into `tray::stop_recording_flow(app)` and call it from both).
  User-started recordings are never auto-stopped.
- Post-stop chain is unchanged: diarization → (Part 4 names) → (Part 2 voice matching) → auto summary.

### Setting

`meeting_detection: "off" | "ask" | "auto"`, default `"ask"`, stored with the existing recording
preferences (`get_recording_preferences` / save). UI: a select in **Settings → Recordings**,
hidden on non-Windows.

---

## Part 2: Voice memory

### Storage

New migration `src-tauri/migrations/20260926000000_add_voice_samples.sql`:

```sql
CREATE TABLE IF NOT EXISTS voice_samples (
    meeting_id TEXT NOT NULL,
    label      TEXT NOT NULL,          -- current speaker name in that meeting
    embedding  BLOB NOT NULL,          -- 256 x f32 little-endian, L2-normalised
    speech_secs REAL NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (meeting_id, label),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_voice_samples_label ON voice_samples(label);
```

- A **known voice** = a label that does not match `^Speaker \d+$`. No separate people table.
- One sample per speaker per meeting = that cluster's centroid (normalised sum of its member
  embeddings, which `identify` already computes). Clusters with < **20 s** of speech are not stored.
- Meeting deletion: the explicit transcript delete in `database/repositories/meeting.rs` also
  deletes the meeting's `voice_samples` (FK cascade is the backstop).
- Re-running identification on a meeting replaces that meeting's samples.

### Matching (inside `identify`, after clustering and after Part 4 naming)

1. Load profiles: for each known label, mean of its samples, re-normalised (skip the current meeting).
2. For every still-unnamed cluster centroid, cosine against every profile.
3. Greedy one-to-one: take the highest (cluster, profile) score ≥ `VOICE_MATCH_THRESHOLD`,
   assign, remove both, repeat.
4. Unmatched clusters keep `Speaker N` (N numbered by first appearance, as today).
5. Write this meeting's samples with the final labels.

`VOICE_MATCH_THRESHOLD = 0.55` (stricter than in-meeting `SAME_SPEAKER_THRESHOLD = 0.40`;
a wrong name is worse than `Speaker 3`). Calibration knob, validated per the Testing section.

### Teaching and correcting

`api_rename_speaker(meeting, from, to)` additionally runs
`UPDATE voice_samples SET label = ? WHERE meeting_id = ? AND label = ?` in the same transaction.
If `to` already exists in the meeting (merge), the two samples are merged: weighted by
`speech_secs`, re-normalised, one row kept.

So "Speaker 2 → Priya" enrols Priya; fixing a wrong match "Priya → Dana" moves the sample
and stops it polluting Priya's profile.

### Settings → Speakers

New tab `speakers` in `src/app/settings/page.tsx` (deep-link `?tab=speakers`):

- List of known voices: name, number of meetings, last heard (max `created_at`).
- **Rename** → `api_rename_voice(from, to)`: updates `voice_samples.label` and
  `transcripts.speaker` everywhere (merging if `to` exists).
- **Forget** → `api_forget_voice(name)`: deletes all samples for the label (transcripts keep the name).
- Note text: "Voiceprints are stored only on this computer."

New commands: `api_list_voices`, `api_rename_voice`, `api_forget_voice`, registered in `lib.rs`.

---

## Part 3: Summary, action items, errors

- No new summary code. Detector-started recordings go through the same stop flow as the tray,
  which navigates to `/meeting-details?id=…&source=recording`, where auto summary waits for
  diarization then generates with the default `standard_meeting` template
  (Summary, Key Decisions, Action Items, Discussion Highlights).
- The main webview stays alive while hidden in the tray, so this runs without the window shown.
  **Verified in the manual smoke test with the window hidden**; only if it fails is the trigger
  moved into Rust (follow-up, not in this plan).

| Situation | Behaviour |
|---|---|
| Capability store unreadable | detector logs once, disables itself; manual recording unaffected |
| Recording already active | no prompt; nothing auto-stopped |
| Two meeting apps hold the mic | one meeting until both release |
| Start fails (e.g. no transcription model) | prompt window shows the error |
| Diarization fails | speakers stay `Speaker N`, summary still runs (existing) |
| No summary model / auto-summary off | prompt shows the hint; transcript + speakers still produced |
| App quits mid-meeting | existing recording recovery |

---

## Part 4: Speaker names from the Zoom / Teams window (hint)

### Step 0: feasibility spike (go/no-go)

`src-tauri/examples/uia_dump.rs`: finds the Zoom / Teams top-level windows and dumps their
UI Automation tree (control type, name, automation id, class, `IsOffscreen`), filtered to nodes
whose name contains `speaking`, `talking`, `muted`, `unmuted`, or that sit in participant /
caption containers. The user runs it during a real Zoom call and a real Teams call.
Anonymised dumps are committed as fixtures under `src-tauri/tests/fixtures/uia/`.

**If neither app exposes a readable speaking indicator, Part 4 is dropped** and Parts 1-3 ship.
The reader rules below are then written against what the dumps actually show.

### Watcher

`src-tauri/src/speaker_hints.rs`, Windows only, UI Automation via the `windows` crate
(already in the dependency tree, 0.61; add features `Win32_UI_Accessibility`,
`Win32_System_Com`, `Win32_System_Registry`, `Win32_UI_WindowsAndMessaging`).

- Runs while a recording is active **and** the detector's current meeting app is Zoom or Teams.
- Every **500 ms**: the per-app reader (`fn zoom_speaking(root) -> Vec<String>`,
  `fn teams_speaking(root) -> Vec<String>`) returns names currently marked speaking.
  Teams live-caption lines (`Name: text`) also count as that name speaking.
- Coalesces into intervals `{name, start, end}` in seconds relative to recording start
  (gap < 1.5 s merges). Written to `speaker-hints.json` in the meeting folder on stop.
- Minimised / unreadable window = no intervals for that stretch. Reader failures log once at `warn`.

### Naming (inside `identify`, after clustering, before Part 2 matching)

For each cluster, overlap seconds of its speech with each hinted name's intervals.
Greedy one-to-one by overlap: assign name if it covers **≥ 60 %** of the cluster's speech and
**≥ 10 s**. Hints win over voice matching; Part 2 fills the rest. Samples saved in Part 2 carry
these names, so voices are learned with no manual renaming.

Only names and times are stored, in the meeting folder (deleted with the meeting).

---

## Testing

**Rust unit tests (no audio, no network):**
- Detector state machine: 5 s start, 15 s end grace, mute blip inside grace, two apps overlapping, browser without meeting title ignored.
- Capability-store entry parsing (`LastUsedTimeStart/Stop` → holding) and exe-path → app mapping, from literal values.
- Browser title matcher: `Meet - abc-defg-hij` yes, `YouTube` no.
- Voice matching: threshold boundary, one-to-one, close runner-up, empty profile set.
- Rename / merge / forget on `voice_samples` against an in-memory SQLite with migrations applied.
- Hint overlap scoring: 60 % boundary, 10 s floor, one-to-one, empty and overlapping intervals.
- Per-app UIA readers against the Step 0 fixtures.

**Calibration:** extend the existing `real_meeting_folder` test (AMI ES2004a): enrol voices from
the first half, identify the second half, report correct-name rate and false-name rate across
thresholds 0.45-0.70; pick `VOICE_MATCH_THRESHOLD` with zero false names if possible.

**Frontend:** `tsc --noEmit`, colour guard, build (CI `frontend-check`).

**Manual smoke (user, Windows):**
1. Join a Zoom or Meet test call → prompt appears within ~5 s → Record.
2. Leave the call → recording stops ~15 s later → speakers labelled → summary with action items appears, with the Noetis window hidden in the tray.
3. Rename one speaker; second call with the same person → name applied automatically.
4. Teams or Zoom call (Part 4) → names applied with no renaming.

## Build order

1. Part 1 (detector + prompt + setting)
2. Part 2 (voice memory + Speakers settings)
3. Part 3 checks (flows verified; no new code expected)
4. Part 4 Step 0 spike (can run alongside 1-2, needs the user in real calls)
5. Part 4 watcher + naming, only if Step 0 passes
