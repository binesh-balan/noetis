# Noetis UI Redesign — Design Spec

Date: 2026-09-24 · Branch: `enhance/ui-redesign` (from `security-hardening`, which carries the managed-policy code the UI depends on and is 29 commits ahead of `main`).

## Decisions (made during brainstorming)

| Question | Decision |
|---|---|
| Scope | Visual refresh **and** layout/navigation restructure |
| Visual direction | "Crisp pro tool": dense, cool neutrals, single indigo accent, keyboard hints (Linear/Raycast feel) |
| Layout | Single sidebar + split detail; recording docked as a bottom bar; ⌘K palette |
| Recording screen | Right pane = **live status pane** (levels, devices, model, warnings); becomes the Summary pane after stop |
| Theme | Dark + light, default follows OS; System/Dark/Light toggle in Settings → Appearance |
| Approach | Tokens first → migrate hard-coded colours → shell → screens, one shippable commit per step |
| Settings | In the main area with a left sub-nav (replaces the horizontal tab strip) |
| Onboarding | Restyled only; same four steps |

Out of scope: new features (live notes, recent-meetings dashboard, full-text search in ⌘K), behaviour changes to recording/transcription/summary, Rust/Tauri changes.

## 1. Tokens and theme

The shadcn token names in `frontend/src/app/globals.css` stay the same; only the values change (hex shown, stored as HSL triplets as today).

| Token | Dark | Light |
|---|---|---|
| `--background` | `#0e0f12` | `#fbfbfc` |
| `--foreground` | `#e6e7ea` | `#16171a` |
| `--card`, `--popover`, `--sidebar` (new) | `#131418` | `#f4f4f6` |
| `--muted`, `--accent`, `--secondary` | `#1d1f25` | `#ececf0` |
| `--muted-foreground` | `#8a8d97` | `#5f6270` |
| `--border` (decorative) | `#22242a` | `#e3e4e8` |
| `--input` (form control outlines) | `#5a5d68` | `#8e919b` |
| `--primary` / `--ring` | `#8b8cf7` | `#5b5bd6` |
| `--primary-foreground` | `#0e0f12` | `#ffffff` |
| `--destructive`, `--recording` (new) | `#f2555a` | `#ce2c31` |
| `--destructive-foreground` | `#0e0f12` | `#ffffff` |
| `--success` (new) | `#4fd1a5` | `#18794e` |
| `--speaker-1..6` (new) | `#8b8cf7 #4fd1a5 #f5a524 #f47fb5 #5cc8ff #c39cff` | `#5b5bd6 #18794e #a35200 #c2297a #0b6fad #7a4bd6` |

Measured contrast (WCAG): dark fg/bg 15.5, dark muted-fg on muted 4.97, dark primary on bg 6.56, dark primary-fg on primary 6.56, dark recording on muted 4.88; light fg/bg 17.3, light muted-fg on muted 5.14, light primary on muted 4.56, white on primary 5.37, light destructive on bg 5.04, white on destructive 5.21. Decorative `--border` is exempt (non-interactive dividers); form outlines use `--input` (≥ 3:1). A contrast check script (`frontend/scripts/check-contrast.mjs`) asserts every pair.

- `--radius`: `0.375rem`.
- Font: Inter via `next/font/google` (bundled at build time, no runtime requests), replacing Source Sans 3.
- Theme: `useTheme` hook (`src/hooks/useTheme.ts`, no new dependency) stores `system|dark|light` in `localStorage` (try/catch), listens to `prefers-color-scheme`, toggles `dark` on `<html>`. An inline script in `layout.tsx` applies the class before first paint.
- BlockNote editor receives `theme={resolvedTheme}`.

## 2. Hard-coded colour migration

1,161 palette classes across 67 `.tsx` files. Mechanical mapping (scripted, then reviewed):

| From | To |
|---|---|
| `bg-white` | `bg-background` (or `bg-card` for raised surfaces) |
| `bg-gray-50/100` | `bg-muted` |
| `bg-gray-200+` | `bg-accent` |
| `text-gray-400..600`, `text-slate-*` equivalents | `text-muted-foreground` |
| `text-gray-700..900`, `text-black` | `text-foreground` |
| `border-gray-*` | `border-border` |
| `*-blue-*` | `primary` variants (`bg-primary/10` for tints) |
| `*-red-*` | `destructive` variants |
| `*-green-*` | `success` variants |
| `*-yellow-*`, `*-amber-*` | `bg-accent` / `text-foreground` (search highlight, warnings) |

Guard: `frontend/scripts/check-colors.mjs` fails if raw palette classes (`(bg|text|border|ring|fill|stroke)-(gray|slate|zinc|neutral|blue|red|green|yellow|amber|white|black)`) appear in `src/`, except a justified allow-list in the script. Wired into `pnpm lint`.

## 3. App shell

`layout.tsx` renders a flex row: `[sidebar | main]`, both full height. The sidebar is no longer `position: fixed`; the 20 `sidebarCollapsed ? '4rem' : '16rem'` margin compensations are removed.

**Sidebar** (`components/Sidebar/index.tsx`, rewritten in place; `SidebarProvider` unchanged):
header (Noetis + ⌘K hint) → New recording / Recording… → Import audio → transcript search (existing, match snippet restyled) → meetings grouped Today / Yesterday / This week / Earlier with hover rename/delete → Settings pinned bottom. Collapses to a 48 px icon strip (`⌘\`).

**RecordingBar** (`app/_components/RecordingBar.tsx`): docked at the bottom of the Home page while recording or processing: dot, elapsed time, level wave, pause/resume, stop. Wraps existing `RecordingControls`; absorbs `StatusOverlays` (processing/saving) as a progress state.

*Revised after reading the code:* the start/stop engine (`useRecordingStart`, `useRecordingStop`, `useRecordingStateSync`: ~960 lines) is mounted by the Home page, and starting from other routes already works via the `autoStartRecording` session flag. Hoisting it into the layout would be a behaviour change, so the bar stays on Home. On other routes the sidebar's top action becomes a live **● Recording · 12:04** item (from `RecordingStateContext.activeDuration`) that navigates to `/`. Stopping still happens on Home, as today.

**Command palette** (`components/CommandPalette.tsx`, uses existing `ui/command.tsx`): new/stop recording, import audio, jump to meeting (titles already in `SidebarProvider`), settings, theme.

**Shortcuts** (one `useHotkeys` effect): `⌘/Ctrl+K` palette, `⌘/Ctrl+R` start recording (same path as the sidebar button; while recording it opens `/`), `⌘/Ctrl+\` sidebar, `⌘/Ctrl+,` settings.

Routes unchanged: `/`, `/meeting-details?id=`, `/settings`.

## 4. Screens

1. **Home, idle:** centred empty state: Start recording (⌘R), Import audio, one line summarising mic / system / model / local-vs-cloud with "Change" → Settings › Recordings.
2. **Home, recording:** `MeetingDetailsSplitView`-style resizable split. Left: live transcript (existing `TranscriptPanel`, speaker colours, in-progress line italic + caret, editable title). Right: `LiveStatusPane` (device names, transcription provider/model, language, paused/active state, and inline warnings: permissions, Bluetooth playback, chunk progress).

*Revised after reading the code:* no audio-level signal exists during recording. The bars in `RecordingControls` are driven by `Math.random()` in `page.tsx`, and the real `audio-levels` monitor (`start_audio_level_monitoring`) opens its own device streams and is only used in device setup. Real live meters need Rust work, so they are out of scope; the pane shows no fake meters, and the bottom bar keeps the existing decorative activity bars.
3. **Meeting details:** same split; single thin toolbar (title · Copy · Export · Retranscribe · Generate/Update summary as compact icon+label buttons, logic in existing `*ButtonGroup` files unchanged); 1 px divider, primary on hover/drag; audio player as slim footer of the transcript pane.
4. **Settings:** left sub-nav (General, Recordings, Transcription, Summary, Templates, Beta, **Appearance**); managed-policy hiding of Transcription preserved. `SettingsModal.tsx` audited: alerts/errors stay dialogs, model-settings modals route to the matching settings section.
5. **Onboarding:** tokens, theme, centred narrow card. Steps unchanged.

Unchanged: transcript virtualisation, transcript recovery, managed-policy hiding, update/analytics prompts (restyled via tokens only).

## 5. Verification

- `pnpm lint` (includes colour guard), `tsc --noEmit`, `pnpm build` after every step.
- `node scripts/check-contrast.mjs` after token changes.
- Screenshots of every screen in both themes via `pnpm run tauri:dev`.
- Manual recording smoke test before merging step 3 and step 4a: 2-minute mic + system recording, pause/resume, stop → saved → details; generate summary; import audio; kill app mid-recording → recovery dialog appears.

## 6. Delivery order

| Step | Content |
|---|---|
| 1 | Tokens, Inter, `useTheme`, no-flash script, Appearance setting, contrast script |
| 2 | Colour migration + colour guard |
| 3 | Shell: grid, sidebar, RecordingBar, ⌘K, shortcuts, margin hacks removed |
| 4a | Home idle + recording with live status pane |
| 4b | Meeting details |
| 4c | Settings sub-nav + `SettingsModal` audit |
| 4d | Onboarding restyle |

Risk: step 4a restructures `page.tsx` around the recording hooks. Mitigation: the hooks and `RecordingControls` internals are untouched (only JSX around them changes); the smoke test gates the merge.
