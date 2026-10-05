# Noetis UI Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle the Noetis desktop frontend in the "crisp pro tool" direction (dark + light) and restructure it into a single-sidebar, split-view layout, with no behaviour change.

**Architecture:** Colour tokens in `globals.css` drive everything; hard-coded Tailwind palette classes are migrated to tokens and banned by a lint guard. The shell becomes a plain flex row (sidebar | main); screens reuse `MeetingDetailsSplitView`. Recording hooks stay mounted by the Home page.

**Tech Stack:** Next.js 14 (app router, static export inside Tauri 2), React 18, Tailwind 3 + shadcn/Radix, `cmdk`, `lucide-react`, BlockNote (`@blocknote/shadcn`).

Spec: `docs/superpowers/specs/2026-09-24-ui-redesign-design.md`

## Global Constraints

- All work under `frontend/`. No Rust/Tauri changes. No new npm dependencies.
- No behaviour changes: Tauri `invoke`/`listen` calls, contexts, hooks' logic, routes (`/`, `/meeting-details?id=`, `/settings`) stay the same.
- Colours only via tokens (`bg-background`, `text-muted-foreground`, `bg-primary`, `text-destructive`, `bg-sidebar`, `text-recording`, `text-success`, `speaker-1..6`, …). No raw palette classes after Task 2.
- Every task ends green on: `cd frontend && npx tsc --noEmit && pnpm lint && pnpm build`.
- `localStorage` access wrapped in try/catch.
- Naming: "microphone" and "system" for audio devices.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

---

### Task 1: Tokens, theme, font, Appearance setting

**Files:**
- Modify: `frontend/src/app/globals.css` (the two `@layer base { :root {…} .dark {…} }` blocks)
- Modify: `frontend/tailwind.config.js` (colors, radius, font)
- Modify: `frontend/src/app/layout.tsx` (font, `<html>` class script)
- Create: `frontend/src/hooks/useTheme.ts`
- Create: `frontend/src/components/AppearanceSettings.tsx`
- Modify: `frontend/src/app/settings/page.tsx` (add Appearance tab; replaced by sub-nav in Task 6)
- Modify: `frontend/src/components/BlockNoteEditor/Editor.tsx:50`
- Create: `frontend/scripts/check-contrast.mjs`

**Interfaces:**
- Produces: `useTheme(): { theme: 'system'|'dark'|'light'; resolvedTheme: 'dark'|'light'; setTheme(t) }`; tokens `sidebar`, `recording`, `success`, `speaker-1`…`speaker-6` as Tailwind colours.

- [ ] **Step 1: Contrast check script (the test)**

`frontend/scripts/check-contrast.mjs`:
```js
// Asserts WCAG contrast for the palette in docs/superpowers/specs/2026-09-24-ui-redesign-design.md.
const lum = (hex) => {
  const c = [0, 2, 4].map((i) => parseInt(hex.slice(1).slice(i, i + 2), 16) / 255)
    .map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const dark = { bg: '#0e0f12', fg: '#e6e7ea', card: '#131418', muted: '#1d1f25', mfg: '#8a8d97', input: '#5a5d68', primary: '#8b8cf7', pfg: '#0e0f12', destr: '#f2555a', dfg: '#0e0f12', success: '#4fd1a5',
  speakers: ['#8b8cf7', '#4fd1a5', '#f5a524', '#f47fb5', '#5cc8ff', '#c39cff'] };
const light = { bg: '#fbfbfc', fg: '#16171a', card: '#f4f4f6', muted: '#ececf0', mfg: '#5f6270', input: '#8e919b', primary: '#5b5bd6', pfg: '#ffffff', destr: '#ce2c31', dfg: '#ffffff', success: '#18794e',
  speakers: ['#5b5bd6', '#18794e', '#a35200', '#c2297a', '#0b6fad', '#7a4bd6'] };
let failed = 0;
const need = (name, a, b, min) => { const r = ratio(a, b); if (r < min) { failed++; console.error(`FAIL ${name}: ${r.toFixed(2)} < ${min}`); } };
for (const [mode, t] of Object.entries({ dark, light })) {
  for (const surf of ['bg', 'card', 'muted']) {
    need(`${mode} fg/${surf}`, t.fg, t[surf], 4.5);
    need(`${mode} muted-fg/${surf}`, t.mfg, t[surf], 4.5);
    need(`${mode} primary/${surf}`, t.primary, t[surf], 4.5);
    need(`${mode} destructive/${surf}`, t.destr, t[surf], 4.5);
    t.speakers.forEach((s, i) => need(`${mode} speaker-${i + 1}/${surf}`, s, t[surf], 4.5));
  }
  need(`${mode} success/bg`, t.success, t.bg, 4.5);
  need(`${mode} primary-fg/primary`, t.pfg, t.primary, 4.5);
  need(`${mode} destructive-fg/destructive`, t.dfg, t.destr, 4.5);
  need(`${mode} input/bg`, t.input, t.bg, 3);
}
if (failed) process.exit(1);
console.log('contrast OK');
```

- [ ] **Step 2: Run it**

Run: `cd frontend && node scripts/check-contrast.mjs`
Expected: `contrast OK`. If any pair fails, darken (light) or lighten (dark) that colour in both the script and the spec until it passes; the script is the source of truth for hex values used below.

- [ ] **Step 3: Replace tokens in `globals.css`**

Replace the `:root { … }` and `.dark { … }` blocks with:
```css
  :root {
    --background: 240 14.3% 98.6%;
    --foreground: 225 8.3% 9.4%;
    --card: 240 10% 96.1%;
    --card-foreground: 225 8.3% 9.4%;
    --popover: 240 14.3% 98.6%;
    --popover-foreground: 225 8.3% 9.4%;
    --sidebar: 240 10% 96.1%;
    --primary: 240 60% 59.8%;
    --primary-foreground: 0 0% 100%;
    --secondary: 240 11.8% 93.3%;
    --secondary-foreground: 225 8.3% 9.4%;
    --muted: 240 11.8% 93.3%;
    --muted-foreground: 229 8.2% 40.6%;
    --accent: 240 11.8% 93.3%;
    --accent-foreground: 225 8.3% 9.4%;
    --destructive: 358 64.8% 49%;
    --destructive-foreground: 0 0% 100%;
    --recording: 358 64.8% 49%;
    --success: 153 66.9% 28.4%;
    --border: 228 9.8% 90%;
    --input: 226 6.1% 58.2%;
    --ring: 240 60% 59.8%;
    --radius: 0.375rem;
    --speaker-1: 240 60% 59.8%;
    --speaker-2: 153 66.9% 28.4%;
    --speaker-3: 30 100% 32%;
    --speaker-4: 328 65.1% 46.1%;
    --speaker-5: 203 88% 36.1%;
    --speaker-6: 260 62.9% 56.7%;
    --chart-1: 240 60% 59.8%;
    --chart-2: 153 66.9% 28.4%;
    --chart-3: 30 100% 32%;
    --chart-4: 328 65.1% 46.1%;
    --chart-5: 203 88% 36.1%;
  }

  .dark {
    --background: 225 12.5% 6.3%;
    --foreground: 225 8.7% 91%;
    --card: 228 11.6% 8.4%;
    --card-foreground: 225 8.7% 91%;
    --popover: 228 11.6% 8.4%;
    --popover-foreground: 225 8.7% 91%;
    --sidebar: 228 11.6% 8.4%;
    --primary: 239 87.1% 75.7%;
    --primary-foreground: 225 12.5% 6.3%;
    --secondary: 225 12.1% 12.9%;
    --secondary-foreground: 225 8.7% 91%;
    --muted: 225 12.1% 12.9%;
    --muted-foreground: 226 5.9% 56.7%;
    --accent: 225 12.1% 12.9%;
    --accent-foreground: 225 8.7% 91%;
    --destructive: 358 85.8% 64.1%;
    --destructive-foreground: 225 12.5% 6.3%;
    --recording: 358 85.8% 64.1%;
    --success: 160 58.6% 56.5%;
    --border: 225 10.5% 14.9%;
    --input: 227 7.2% 38%;
    --ring: 239 87.1% 75.7%;
    --speaker-1: 239 87.1% 75.7%;
    --speaker-2: 160 58.6% 56.5%;
    --speaker-3: 37 91.3% 55.1%;
    --speaker-4: 332 84.2% 72.7%;
    --speaker-5: 200 100% 68%;
    --speaker-6: 264 100% 80.6%;
    --chart-1: 239 87.1% 75.7%;
    --chart-2: 160 58.6% 56.5%;
    --chart-3: 37 91.3% 55.1%;
    --chart-4: 332 84.2% 72.7%;
    --chart-5: 200 100% 68%;
  }
```
In the second `@layer base` block, make sure `body` has `@apply bg-background text-foreground;` and add `color-scheme: light;` on `:root` and `color-scheme: dark;` on `.dark` (native scrollbars/inputs follow the theme).

- [ ] **Step 4: Tailwind config**

In `frontend/tailwind.config.js` `theme.extend`:
```js
fontFamily: { sans: ['var(--font-inter)', 'system-ui', 'sans-serif'] },
```
and inside `colors` add:
```js
sidebar: 'hsl(var(--sidebar))',
recording: 'hsl(var(--recording))',
success: 'hsl(var(--success))',
speaker: {
  '1': 'hsl(var(--speaker-1))', '2': 'hsl(var(--speaker-2))', '3': 'hsl(var(--speaker-3))',
  '4': 'hsl(var(--speaker-4))', '5': 'hsl(var(--speaker-5))', '6': 'hsl(var(--speaker-6))',
},
```
Delete `tertiary: '#64748b'`; replace its usages (`grep -rn "tertiary" frontend/src`) with `muted-foreground`.

- [ ] **Step 5: `useTheme` hook**

`frontend/src/hooks/useTheme.ts`:
```ts
'use client';
import { useCallback, useEffect, useState } from 'react';

export type ThemeChoice = 'system' | 'dark' | 'light';
const KEY = 'noetis-theme';
const media = () => window.matchMedia('(prefers-color-scheme: dark)');

function read(): ThemeChoice {
  try {
    const v = localStorage.getItem(KEY);
    return v === 'dark' || v === 'light' ? v : 'system';
  } catch {
    return 'system';
  }
}

function apply(choice: ThemeChoice): 'dark' | 'light' {
  const resolved = choice === 'system' ? (media().matches ? 'dark' : 'light') : choice;
  document.documentElement.classList.toggle('dark', resolved === 'dark');
  return resolved;
}

export function useTheme() {
  const [theme, setThemeState] = useState<ThemeChoice>('system');
  const [resolvedTheme, setResolved] = useState<'dark' | 'light'>('dark');

  useEffect(() => {
    const initial = read();
    setThemeState(initial);
    setResolved(apply(initial));
    const onChange = () => setResolved(apply(read()));
    const mq = media();
    mq.addEventListener('change', onChange);
    // Keep every useTheme instance in sync when another one changes the choice.
    window.addEventListener('noetis-theme-change', onChange);
    return () => {
      mq.removeEventListener('change', onChange);
      window.removeEventListener('noetis-theme-change', onChange);
    };
  }, []);

  const setTheme = useCallback((next: ThemeChoice) => {
    try {
      if (next === 'system') localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, next);
    } catch {
      /* storage unavailable: theme still applies for this session */
    }
    setThemeState(next);
    setResolved(apply(next));
    window.dispatchEvent(new Event('noetis-theme-change'));
  }, []);

  return { theme, resolvedTheme, setTheme };
}

// Inline, runs before first paint (see layout.tsx). Must mirror read()/apply().
export const THEME_BOOT_SCRIPT = `(function(){try{var v=localStorage.getItem('${KEY}');var d=v==='dark'||(v!=='light'&&window.matchMedia('(prefers-color-scheme: dark)').matches);document.documentElement.classList.toggle('dark',d);}catch(e){document.documentElement.classList.add('dark');}})();`;
```

- [ ] **Step 6: Layout font + boot script**

In `frontend/src/app/layout.tsx`: replace the `Source_Sans_3` import/const with
```tsx
import { Inter } from 'next/font/google'
const inter = Inter({ subsets: ['latin'], variable: '--font-inter' })
```
import `THEME_BOOT_SCRIPT` from `@/hooks/useTheme`, and change the `<html>`/`<body>` opening to:
```tsx
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: THEME_BOOT_SCRIPT }} />
      </head>
      <body className={`${inter.variable} font-sans antialiased bg-background text-foreground`}>
```
Change `<Toaster … richColors …/>` to also pass `theme="system"`. Check `frontend/src-tauri/tauri.conf.json` for a CSP: if `script-src` exists without `'unsafe-inline'`, add the script's SHA-256 hash (`node -e "console.log('sha256-'+require('crypto').createHash('sha256').update(require('./src/hooks/useTheme').THEME_BOOT_SCRIPT ?? '').digest('base64'))"` won't work on TS, so compute it from the literal string) to `script-src`, keeping all existing entries.

- [ ] **Step 7: Appearance settings + BlockNote theme**

`frontend/src/components/AppearanceSettings.tsx`:
```tsx
'use client';
import { Monitor, Moon, Sun } from 'lucide-react';
import { useTheme, type ThemeChoice } from '@/hooks/useTheme';

const OPTIONS: { value: ThemeChoice; label: string; icon: typeof Sun }[] = [
  { value: 'system', label: 'System', icon: Monitor },
  { value: 'dark', label: 'Dark', icon: Moon },
  { value: 'light', label: 'Light', icon: Sun },
];

export function AppearanceSettings() {
  const { theme, setTheme } = useTheme();
  return (
    <section className="space-y-3">
      <div>
        <h3 className="text-sm font-semibold">Theme</h3>
        <p className="text-sm text-muted-foreground">System follows your operating system setting.</p>
      </div>
      <div role="radiogroup" aria-label="Theme" className="inline-flex rounded-md border border-border bg-card p-0.5">
        {OPTIONS.map(({ value, label, icon: Icon }) => (
          <button
            key={value}
            role="radio"
            aria-checked={theme === value}
            onClick={() => setTheme(value)}
            className={`flex items-center gap-1.5 rounded px-3 py-1.5 text-sm transition-colors ${
              theme === value ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground'
            }`}
          >
            <Icon className="h-4 w-4" />
            {label}
          </button>
        ))}
      </div>
    </section>
  );
}
```
In `settings/page.tsx` add `{ value: 'appearance', label: 'Appearance', icon: Palette }` to `TABS` (import `Palette` from lucide) and a `<TabsContent value="appearance" className="mt-6"><AppearanceSettings /></TabsContent>`.

In `BlockNoteEditor/Editor.tsx` replace `theme="light"` with `theme={resolvedTheme}` using `const { resolvedTheme } = useTheme();`.

- [ ] **Step 8: Verify**

Run: `cd frontend && node scripts/check-contrast.mjs && npx tsc --noEmit && pnpm lint && pnpm build`
Expected: all pass. Launch `pnpm run tauri:dev`, toggle Settings → Appearance between Dark/Light/System: background and text flip; no white flash on reload in dark.

- [ ] **Step 9: Commit**

```bash
git add frontend && git commit -m "feat(ui): pro-tool colour tokens, Inter, light/dark theme with Appearance setting"
```

---

### Task 2: Migrate hard-coded colours + guard

**Files:**
- Create: `frontend/scripts/check-colors.mjs`
- Modify: `frontend/package.json` (`lint` script)
- Modify: all `.tsx` under `frontend/src` that contain palette classes (67 files)
- Scratch (not committed): `migrate-colors.mjs` in the session scratchpad

**Interfaces:**
- Consumes: tokens from Task 1.
- Produces: `pnpm lint` fails on raw palette classes.

- [ ] **Step 1: Write the guard (the test)**

`frontend/scripts/check-colors.mjs`:
```js
// Fails when raw Tailwind palette classes are used instead of theme tokens.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

const RAW = /\b(?:[a-z-]+:)*(?:bg|text|border|ring|fill|stroke|from|to|via|divide|outline|placeholder|decoration|accent|caret|shadow)-(?:gray|slate|zinc|neutral|stone|blue|sky|indigo|red|rose|green|emerald|yellow|amber|orange|purple|violet|pink|teal|cyan|lime|white|black)(?:-\d{2,3})?(?:\/\d+)?\b/g;
// Paths allowed to keep raw colours, with the reason.
const ALLOW = new Map([
  // ['src/components/Example.tsx', 'third-party brand colour'],
]);

const files = [];
const walk = (d) => readdirSync(d).forEach((f) => {
  const p = join(d, f);
  if (statSync(p).isDirectory()) walk(p);
  else if (/\.(tsx?|jsx?)$/.test(f)) files.push(p);
});
walk('src');

let hits = 0;
for (const f of files) {
  const rel = f.replaceAll('\\', '/');
  if (ALLOW.has(rel)) continue;
  readFileSync(f, 'utf8').split('\n').forEach((line, i) => {
    const m = line.match(RAW);
    if (m) { hits += m.length; console.error(`${rel}:${i + 1}  ${m.join(' ')}`); }
  });
}
if (hits) { console.error(`\n${hits} raw colour classes. Use theme tokens (see globals.css).`); process.exit(1); }
console.log('colours OK');
```
In `package.json` change `"lint": "next lint"` to `"lint": "next lint && node scripts/check-colors.mjs"`.

- [ ] **Step 2: Run it to see it fail**

Run: `cd frontend && node scripts/check-colors.mjs | tail -3`
Expected: FAIL, ~1,161 raw colour classes.

- [ ] **Step 3: Codemod (scratchpad, not committed)**

`<scratchpad>/migrate-colors.mjs`: run with `node <scratchpad>/migrate-colors.mjs` from `frontend/`:
```js
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
// Order matters: most specific first. Variant prefixes (hover:, dark:, …) are preserved by matching the class stem only.
const MAP = [
  [/\bbg-white\b/g, 'bg-background'], [/\bbg-black\b/g, 'bg-foreground'],
  [/\btext-white\b/g, 'text-primary-foreground'], [/\btext-black\b/g, 'text-foreground'],
  [/\bbg-(?:gray|slate|zinc|neutral)-(?:50|100)\b/g, 'bg-muted'],
  [/\bbg-(?:gray|slate|zinc|neutral)-(?:200|300)\b/g, 'bg-accent'],
  [/\bbg-(?:gray|slate|zinc|neutral)-(?:[4-9]00)\b/g, 'bg-foreground'],
  [/\btext-(?:gray|slate|zinc|neutral)-(?:[3-6]00)\b/g, 'text-muted-foreground'],
  [/\btext-(?:gray|slate|zinc|neutral)-(?:[7-9]00|950)\b/g, 'text-foreground'],
  [/\b(border|divide|ring|outline)-(?:gray|slate|zinc|neutral)-\d{2,3}\b/g, '$1-border'],
  [/\bbg-(?:blue|indigo|sky)-(?:50|100)\b/g, 'bg-primary/10'],
  [/\bbg-(?:blue|indigo|sky)-(?:200|300)\b/g, 'bg-primary/20'],
  [/\bbg-(?:blue|indigo|sky)-(?:[4-9]00)\b/g, 'bg-primary'],
  [/\b(text|border|ring|outline|fill|stroke|accent|caret)-(?:blue|indigo|sky)-\d{2,3}\b/g, '$1-primary'],
  [/\bbg-(?:red|rose)-(?:50|100)\b/g, 'bg-destructive/10'],
  [/\bbg-(?:red|rose)-(?:200|300)\b/g, 'bg-destructive/20'],
  [/\bbg-(?:red|rose)-(?:[4-9]00)\b/g, 'bg-destructive'],
  [/\b(text|border|ring|outline|fill|stroke)-(?:red|rose)-\d{2,3}\b/g, '$1-destructive'],
  [/\bbg-(?:green|emerald)-(?:50|100)\b/g, 'bg-success/10'],
  [/\bbg-(?:green|emerald)-(?:[2-9]00)\b/g, 'bg-success'],
  [/\b(text|border|ring|fill|stroke)-(?:green|emerald)-\d{2,3}\b/g, '$1-success'],
];
const walk = (d, out = []) => { readdirSync(d).forEach((f) => { const p = join(d, f); statSync(p).isDirectory() ? walk(p, out) : /\.tsx?$/.test(f) && out.push(p); }); return out; };
let changed = 0;
for (const f of walk('src')) {
  const src = readFileSync(f, 'utf8');
  let out = src;
  for (const [re, to] of MAP) out = out.replace(re, to);
  if (out !== src) { writeFileSync(f, out); changed++; }
}
console.log(`rewrote ${changed} files`);
```

- [ ] **Step 4: Run codemod, then fix the remainder by hand**

Run: `node <scratchpad>/migrate-colors.mjs && node scripts/check-colors.mjs | tail -40`
Remaining hits are yellow/amber/orange/purple/etc. and `text-primary-foreground` misfires. For each file:
- `text-primary-foreground` produced from `text-white`: keep it only where the element's background is `bg-primary`; where the background is `bg-destructive` use `text-destructive-foreground`; on `bg-success` use `text-background`; elsewhere (e.g. white text over images/overlays) use `text-foreground`.
- yellow/amber/orange (warnings, search-match highlight): `bg-accent` + `text-foreground`, icons `text-muted-foreground`; true warnings keep emphasis via `border-border` + a `⚠` / lucide `AlertTriangle` icon in `text-destructive`.
- purple/violet/pink/teal/cyan (badges, speaker tags): speaker labels → `text-speaker-N` by index (`N = (index % 6) + 1`); other badges → `bg-primary/10 text-primary`.
- `bg-foreground` produced from dark greys (tooltips, overlays): check the text on it is `text-background`.
- `shadow-*` with colour → plain `shadow-sm`/`shadow-md`.
Rerun until `colours OK`.

- [ ] **Step 5: Verify both themes**

Run: `npx tsc --noEmit && pnpm lint && pnpm build`. Then `pnpm run tauri:dev`; walk Home, Meeting details, every Settings tab, and Onboarding (temporarily via `invoke('reset_onboarding')` if that command exists, else skip) in Dark and Light. Fix any unreadable text or invisible borders (usual culprit: `bg-background` on a surface that should be `bg-card`).

- [ ] **Step 6: Commit**

```bash
git add frontend && git commit -m "refactor(ui): replace hard-coded colours with theme tokens; lint guard against raw palette classes"
```

---

### Task 3: App shell: sidebar, command palette, shortcuts

**Files:**
- Modify: `frontend/src/app/layout.tsx` (shell markup)
- Modify: `frontend/src/components/MainContent/index.tsx`
- Rewrite: `frontend/src/components/Sidebar/index.tsx`
- Create: `frontend/src/components/CommandPalette.tsx`
- Create: `frontend/src/hooks/useHotkeys.ts`
- Modify: every file using `sidebarCollapsed ? '4rem' : '16rem'`/`ml-16`/`ml-64` margin compensation (`grep -rn "sidebarCollapsed\|isCollapsed" frontend/src --include=*.tsx | grep -v components/Sidebar`)

**Interfaces:**
- Consumes: `useSidebar()` (`isCollapsed`, `toggleCollapse` (check the exact setter name in `SidebarProvider.tsx`), `meetings`, `handleRecordingToggle`, `searchTranscripts`, `searchResults`, `isSearching`, `setMeetings`, `refetchMeetings`), `useRecordingState()` (`isRecording`, `isPaused`, `activeDuration`), `useImportDialog().openImportDialog`, `useConfig().betaFeatures`, `useTheme()`.
- Produces: `useHotkeys(map: Record<string, (e: KeyboardEvent) => void>)` with keys like `'mod+k'`; `<CommandPalette open onOpenChange />`; `formatDuration(seconds: number): string` from `frontend/src/lib/formatDuration.ts`.

- [ ] **Step 1: `formatDuration` + self-check**

`frontend/src/lib/formatDuration.ts`:
```ts
/** 65 → "1:05", 3725 → "1:02:05" */
export function formatDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}
```
Check: `node -e "const f=s=>{s=Math.max(0,Math.floor(s));const h=Math.floor(s/3600),m=Math.floor(s%3600/60),x=String(s%60).padStart(2,'0');return h>0?h+':'+String(m).padStart(2,'0')+':'+x:m+':'+x};console.assert(f(65)==='1:05');console.assert(f(3725)==='1:02:05');console.assert(f(-3)==='0:00');console.log('ok')"`. Before using it, confirm the unit of `activeDuration` in `RecordingStateContext.tsx` (seconds vs ms, set from `backendState.active_duration`) and divide by 1000 at the call site if it is ms.

- [ ] **Step 2: `useHotkeys`**

`frontend/src/hooks/useHotkeys.ts`:
```ts
'use client';
import { useEffect, useRef } from 'react';

/** Keys: 'mod+k', 'mod+r', 'mod+\\', 'mod+,'. `mod` = ⌘ on macOS, Ctrl elsewhere. */
export function useHotkeys(map: Record<string, (e: KeyboardEvent) => void>) {
  const ref = useRef(map);
  ref.current = map;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
      const handler = ref.current[`mod+${e.key.toLowerCase()}`];
      if (handler) {
        e.preventDefault();
        handler(e);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}
```
Note: `mod+r` must `preventDefault` to stop the webview reload; that's handled above.

- [ ] **Step 3: `CommandPalette`**

`frontend/src/components/CommandPalette.tsx`: uses the existing `ui/command.tsx` exports (check names: typically `CommandDialog, CommandInput, CommandList, CommandEmpty, CommandGroup, CommandItem, CommandShortcut`):
```tsx
'use client';
import { useRouter } from 'next/navigation';
import { FileText, Mic, Monitor, Moon, Settings, Sun, Upload } from 'lucide-react';
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandShortcut } from '@/components/ui/command';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { useConfig } from '@/contexts/ConfigContext';
import { useTheme } from '@/hooks/useTheme';

export function CommandPalette({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const router = useRouter();
  const { meetings, handleRecordingToggle } = useSidebar();
  const { isRecording } = useRecordingState();
  const { openImportDialog } = useImportDialog();
  const { betaFeatures } = useConfig();
  const { setTheme } = useTheme();
  const run = (fn: () => void) => () => { onOpenChange(false); fn(); };

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange}>
      <CommandInput placeholder="Type a command or meeting name…" />
      <CommandList>
        <CommandEmpty>No results.</CommandEmpty>
        <CommandGroup heading="Actions">
          <CommandItem onSelect={run(() => (isRecording ? router.push('/') : handleRecordingToggle()))}>
            <Mic /> {isRecording ? 'Go to recording' : 'New recording'} <CommandShortcut>⌘R</CommandShortcut>
          </CommandItem>
          {betaFeatures.importAndRetranscribe && (
            <CommandItem onSelect={run(() => openImportDialog())}><Upload /> Import audio</CommandItem>
          )}
          <CommandItem onSelect={run(() => router.push('/settings'))}>
            <Settings /> Settings <CommandShortcut>⌘,</CommandShortcut>
          </CommandItem>
        </CommandGroup>
        <CommandGroup heading="Theme">
          <CommandItem onSelect={run(() => setTheme('system'))}><Monitor /> System theme</CommandItem>
          <CommandItem onSelect={run(() => setTheme('dark'))}><Moon /> Dark theme</CommandItem>
          <CommandItem onSelect={run(() => setTheme('light'))}><Sun /> Light theme</CommandItem>
        </CommandGroup>
        <CommandGroup heading="Meetings">
          {meetings.map((m) => (
            <CommandItem key={m.id} value={`${m.title} ${m.id}`} onSelect={run(() => router.push(`/meeting-details?id=${m.id}`))}>
              <FileText /> {m.title}
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
```
Adjust the `meetings` field names (`id`, `title`) to the actual type in `SidebarProvider.tsx`.

- [ ] **Step 4: Shell in `layout.tsx` and `MainContent`**

Replace `<div className="flex"><Sidebar /><MainContent>{children}</MainContent></div>` with `<AppShell>{children}</AppShell>` where `AppShell` is a module-level component in `layout.tsx` (it must sit inside the providers):
```tsx
function AppShell({ children }: { children: React.ReactNode }) {
  const router = useRouter();
  const { isRecording } = useRecordingState();
  const { handleRecordingToggle, toggleCollapse } = useSidebar();
  const [paletteOpen, setPaletteOpen] = useState(false);
  useHotkeys({
    'mod+k': () => setPaletteOpen((o) => !o),
    'mod+r': () => (isRecording ? router.push('/') : handleRecordingToggle()),
    'mod+\\': () => toggleCollapse(),
    'mod+,': () => router.push('/settings'),
  });
  return (
    <div className="flex h-screen overflow-hidden bg-background text-foreground">
      <Sidebar />
      <MainContent>{children}</MainContent>
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} />
    </div>
  );
}
```
`MainContent` becomes:
```tsx
const MainContent: React.FC<MainContentProps> = ({ children }) => (
  <main className="flex min-w-0 flex-1 flex-col overflow-hidden">{children}</main>
);
```
Remove every margin/padding compensation for the fixed sidebar found by the grep in **Files** (e.g. the `style={{ marginLeft: sidebarCollapsed ? '4rem' : '16rem' }}` in `app/page.tsx` and in `StatusOverlays.tsx`, and any `ml-16`/`ml-64`/`pl-8` offsets). Pages' own root `h-screen` becomes `h-full`.

- [ ] **Step 5: Rewrite `Sidebar/index.tsx`**

Read the whole current file first and keep every behaviour: rename modal (`editModalState`, save via existing handler), delete confirmation (`deleteModalState` + `ConfirmationModal`), transcript search with match snippet, `ModelSettingsModal` wiring if still referenced, `setMeetings` updates after rename/delete, analytics calls, current-meeting highlight, the `NotebookPen` action (check what it does and keep it as a sidebar item). Drop only the icon rail and the fixed positioning. New markup (tokens only):
- `<aside className={cn('flex h-full shrink-0 flex-col border-r border-border bg-sidebar transition-[width] duration-200', isCollapsed ? 'w-12' : 'w-60')}>`
- Header row: "Noetis" `text-sm font-semibold` + `<kbd>` "⌘K" (`rounded border border-border px-1 text-[10px] text-muted-foreground`), collapse button (`PanelLeft` icon, `aria-label="Collapse sidebar"`).
- Primary action: if `!isRecording` → button "New recording" (`Mic` icon in `text-recording`) calling `handleRecordingToggle`; if recording → link to `/` rendering a pulsing dot (`h-2 w-2 rounded-full bg-recording animate-pulse`), "Recording" / "Paused", and `formatDuration(activeDuration)` in `tabular-nums text-muted-foreground`.
- "Import audio" (when `betaFeatures.importAndRetranscribe`).
- Search input (`h-8 bg-background border-input text-sm`), same `handleSearchChange` logic.
- Meetings grouped by date. Add in the file:
  ```ts
  function groupLabel(date: Date, now = new Date()): 'Today' | 'Yesterday' | 'This week' | 'Earlier' {
    const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    const diff = Math.round((day(now) - day(date)) / 86_400_000);
    return diff <= 0 ? 'Today' : diff === 1 ? 'Yesterday' : diff < 7 ? 'This week' : 'Earlier';
  }
  ```
  Use the meeting's created/updated timestamp field from the meeting type; if meetings have no date, render one ungrouped "Meetings" list. Group headers: `px-2 pt-3 text-[10px] font-medium uppercase tracking-wider text-muted-foreground`. Rows: `group flex items-center rounded-md px-2 py-1.5 text-sm text-muted-foreground hover:bg-muted hover:text-foreground`, active row `bg-muted text-foreground`; rename/delete icon buttons appear on `group-hover`, each with `aria-label`.
- Footer: Settings link (`Settings` icon, `⌘,` hint).
- Collapsed (`w-12`): only icons for New recording/Recording dot, Import, Search (expands sidebar on click), Settings, each wrapped in the existing `Tooltip` with `side="right"`.

- [ ] **Step 6: Verify**

Run: `npx tsc --noEmit && pnpm lint && pnpm build`. In `pnpm run tauri:dev`: ⌘/Ctrl+K opens the palette and jumps to a meeting; ⌘/Ctrl+\ collapses; ⌘/Ctrl+, opens settings; ⌘/Ctrl+R starts a recording from `/settings` (navigates home and auto-starts); rename and delete a meeting; search finds transcript text; while recording, open a past meeting and confirm the sidebar shows the live timer and returns you to `/`. Both themes.

- [ ] **Step 7: Commit**

```bash
git add frontend && git commit -m "feat(ui): single-sidebar shell, command palette and keyboard shortcuts"
```

---

### Task 4: Home: idle state, recording split view, recording bar

**Files:**
- Modify: `frontend/src/app/page.tsx` (JSX only; hooks untouched)
- Create: `frontend/src/app/_components/HomeIdle.tsx`
- Create: `frontend/src/app/_components/LiveStatusPane.tsx`
- Create: `frontend/src/app/_components/RecordingBar.tsx`
- Modify: `frontend/src/app/_components/TranscriptPanel.tsx`, `StatusOverlays.tsx` (restyle)
- Modify: `frontend/src/components/RecordingControls.tsx` (markup/classes only)
- Modify: `frontend/src/components/MeetingDetails/MeetingDetailsSplitView.tsx` (divider style; generic labels)

**Interfaces:**
- Consumes: `MeetingDetailsSplitView({ transcript, summary, activeTab, onTabChange })`; `useConfig()` (`selectedDevices`, `transcriptModelConfig`, and the language setting: check its name in `ConfigContext.tsx`); `useRecordingState()`; `formatDuration`.
- Produces: `<HomeIdle onStart={() => void} />`, `<LiveStatusPane />`, `<RecordingBar>{controls}</RecordingBar>`.

- [ ] **Step 1: `HomeIdle`**

```tsx
'use client';
import Link from 'next/link';
import { Mic, Upload } from 'lucide-react';
import { useConfig } from '@/contexts/ConfigContext';
import { useImportDialog } from '@/contexts/ImportDialogContext';

export function HomeIdle({ onStart, disabled }: { onStart: () => void; disabled?: boolean }) {
  const { selectedDevices, transcriptModelConfig, betaFeatures } = useConfig();
  const { openImportDialog } = useImportDialog();
  const mic = selectedDevices?.micDevice ?? 'Default microphone';
  const system = selectedDevices?.systemDevice ?? 'Default system audio';
  return (
    <div className="flex h-full flex-col items-center justify-center gap-5 p-8 text-center">
      <button
        onClick={onStart}
        disabled={disabled}
        aria-label="Start recording"
        className="flex h-16 w-16 items-center justify-center rounded-full border border-recording/60 bg-card transition-colors hover:bg-recording/10 disabled:opacity-50"
      >
        <span className="h-6 w-6 rounded-full bg-recording" />
      </button>
      <div>
        <h1 className="text-lg font-semibold">Start recording</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          <kbd className="rounded border border-border px-1 text-[11px]">⌘R</kbd> from anywhere
        </p>
      </div>
      {betaFeatures.importAndRetranscribe && (
        <button onClick={() => openImportDialog()} className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground">
          <Upload className="h-4 w-4" /> Import audio file
        </button>
      )}
      <p className="max-w-md text-xs text-muted-foreground">
        <Mic className="mr-1 inline h-3 w-3" />
        {mic} · {system} · {transcriptModelConfig.model}{' '}
        <Link href="/settings" className="text-primary hover:underline">Change</Link>
      </p>
    </div>
  );
}
```
`onStart` must trigger the same path as today's record button: dispatch `window.dispatchEvent(new CustomEvent('start-recording-from-sidebar'))` (that is what `useRecordingStart` listens for, `hooks/useRecordingStart.ts:388`) or call `handleRecordingStart` directly from `page.tsx`. Prefer `handleRecordingStart`.

- [ ] **Step 2: `LiveStatusPane`**

```tsx
'use client';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';

const Row = ({ k, v }: { k: string; v: string }) => (
  <div className="flex items-baseline justify-between gap-3 text-sm">
    <span className="text-muted-foreground">{k}</span>
    <span className="truncate text-right" title={v}>{v}</span>
  </div>
);

export function LiveStatusPane({ children }: { children?: React.ReactNode }) {
  const { selectedDevices, transcriptModelConfig } = useConfig();
  const { isPaused } = useRecordingState();
  return (
    <div className="flex h-full flex-col gap-5 overflow-y-auto p-4">
      <section className="space-y-2">
        <h2 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Input</h2>
        <Row k="Microphone" v={selectedDevices?.micDevice ?? 'Default'} />
        <Row k="System" v={selectedDevices?.systemDevice ?? 'Default'} />
        <Row k="State" v={isPaused ? 'Paused' : 'Listening'} />
      </section>
      <section className="space-y-2">
        <h2 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Engine</h2>
        <Row k="Provider" v={transcriptModelConfig.provider} />
        <Row k="Model" v={transcriptModelConfig.model} />
      </section>
      {children /* warnings: PermissionWarning, BluetoothPlaybackWarning, chunk progress */}
      <p className="mt-auto text-xs text-muted-foreground">The summary appears here after you stop.</p>
    </div>
  );
}
```
Add a `Language` row if `ConfigContext` exposes the transcription language (grep `language` in `contexts/ConfigContext.tsx`); use its display name if a helper exists (`LanguageSelection.tsx`).

- [ ] **Step 3: `RecordingBar`**

```tsx
'use client';
export function RecordingBar({ children }: { children: React.ReactNode }) {
  return (
    <div className="shrink-0 border-t border-border bg-card px-4 py-2">
      <div className="mx-auto flex max-w-5xl items-center justify-center">{children}</div>
    </div>
  );
}
```
In `RecordingControls.tsx` restyle markup only: remove the white rounded-full pill wrapper styles in favour of a flat row: record dot `bg-recording`, timer `tabular-nums`, activity bars `bg-primary/60`, Pause/Resume as `border border-border rounded-md px-2.5 py-1 text-sm`, Stop as `bg-primary text-primary-foreground rounded-md px-3 py-1 text-sm font-medium`. Do not touch handlers, effects or state.

- [ ] **Step 4: Recompose `page.tsx` JSX**

Keep every hook and handler as is. Replace the returned JSX with:
```tsx
  const showControls = (hasMicrophone || isRecording) &&
    status !== RecordingStatus.PROCESSING_TRANSCRIPTS && status !== RecordingStatus.SAVING;
  const sessionActive = recordingState.isRecording || isProcessingStop || status === RecordingStatus.SAVING;

  return (
    <div className="flex h-full flex-col bg-background">
      <SettingsModals modals={modals} messages={messages} onClose={hideModal} />
      <TranscriptRecovery
        isOpen={showRecoveryDialog}
        onClose={handleDialogClose}
        recoverableMeetings={recoverableMeetings}
        onRecover={handleRecovery}
        onDelete={deleteRecoverableMeeting}
        onLoadPreview={loadMeetingTranscripts}
      />
      <div className="min-h-0 flex-1">
        {sessionActive ? (
          <MeetingDetailsSplitView
            transcript={<TranscriptPanel isProcessingStop={isProcessingStop} isStopping={isStopping} showModal={showModal} />}
            summary={<LiveStatusPane />}
            activeTab={liveTab}
            onTabChange={setLiveTab}
          />
        ) : (
          <HomeIdle onStart={handleRecordingStart} disabled={isRecordingDisabled || !hasMicrophone} />
        )}
      </div>
      <StatusOverlays
        isProcessing={status === RecordingStatus.PROCESSING_TRANSCRIPTS && !recordingState.isRecording}
        isSaving={status === RecordingStatus.SAVING}
      />
      {showControls && sessionActive && (
        <RecordingBar>
          <RecordingControls
            isRecording={recordingState.isRecording}
            onRecordingStop={(callApi = true) => handleRecordingStop(callApi)}
            onRecordingStart={handleRecordingStart}
            onTranscriptReceived={() => { }}
            onStopInitiated={() => setIsStopping(true)}
            barHeights={barHeights}
            onTranscriptionError={(message) => showModal('errorAlert', message)}
            isRecordingDisabled={isRecordingDisabled}
            isParentProcessing={isProcessingStop}
            selectedDevices={selectedDevices}
            meetingName={meetingTitle}
          />
        </RecordingBar>
      )}
    </div>
  );
```
with `const [liveTab, setLiveTab] = useState<MeetingDetailsTab>('transcript');`. **Check before shipping:** if `RecordingControls` must be mounted *before* recording starts (e.g. it owns the start flow, or listens for `start-recording-from-sidebar`/auto-start), render it hidden (`className="hidden"` wrapper) while idle instead of not mounting it, so start behaviour is identical. Read `RecordingControls.tsx` to decide. Move the `PermissionWarning` that `TranscriptPanel` renders (line ~96) into `LiveStatusPane` children only if it is recording-time relevant; otherwise leave it where it is (it's shown when idle too, so also render it in `HomeIdle` via the same condition).

`StatusOverlays`: drop the `sidebarCollapsed` prop/margins; render as a bottom strip in the same style as `RecordingBar` (`border-t border-border bg-card`, spinner + "Processing transcript…" / "Saving…").

`MeetingDetailsSplitView`: divider line `bg-border`, hover/active `bg-primary`; focus ring `ring-ring`.

- [ ] **Step 5: `TranscriptPanel` restyle**

Speaker labels `text-speaker-N` (`N = (speakerIndex % 6) + 1`, where the index is the order speakers first appear), in-progress text `italic text-muted-foreground` + caret `<span className="ml-0.5 inline-block h-3.5 w-1.5 animate-pulse bg-primary align-middle" />`, title editable as before. Apply the same speaker colouring in `components/TranscriptView.tsx` / `VirtualizedTranscriptView.tsx` if that is where lines render.

- [ ] **Step 6: Verify + smoke test**

`npx tsc --noEmit && pnpm lint && pnpm build`, then the full smoke test from the spec (§5) in `pnpm run tauri:dev`, both themes. Required before commit.

- [ ] **Step 7: Commit**

```bash
git add frontend && git commit -m "feat(ui): home idle state, live split view with status pane, docked recording bar"
```

---

### Task 5: Meeting details

**Files:**
- Modify: `frontend/src/app/meeting-details/page-content.tsx`, `page.tsx` (layout/toolbar)
- Modify: `frontend/src/components/MeetingDetails/{TranscriptPanel,SummaryPanel,TranscriptButtonGroup,SummaryGeneratorButtonGroup,SummaryUpdaterButtonGroup}.tsx` (markup/classes only)
- Modify: `frontend/src/components/AudioPlayer.tsx` (slim footer style)

- [ ] **Step 1: Toolbar**

One row at the top of `page-content.tsx`: `flex h-11 items-center gap-2 border-b border-border px-4`; `EditableTitle` on the left (`text-sm font-semibold`), then `ml-auto` and the button groups. Button look (apply inside the ButtonGroup files, handlers unchanged): `inline-flex h-7 items-center gap-1.5 rounded-md border border-border px-2.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground`, icons `h-3.5 w-3.5`; the primary action (Generate summary) uses `bg-primary text-primary-foreground border-transparent`.

- [ ] **Step 2: Panels**

Transcript and summary panels: remove card chrome (`shadow`, rounded white boxes); content padding `p-4`; section headers `text-[10px] uppercase tracking-wider text-muted-foreground`. Audio player: `border-t border-border bg-card px-3 py-2` at the bottom of the transcript pane.

- [ ] **Step 3: Verify**

`npx tsc --noEmit && pnpm lint && pnpm build`; open a meeting in both themes; copy, export, retranscribe dialog opens, generate summary, resize split, narrow window switches to tabs.

- [ ] **Step 4: Commit**

```bash
git add frontend && git commit -m "feat(ui): meeting details toolbar and panels in the new style"
```

---

### Task 6: Settings sub-nav + modal audit

**Files:**
- Modify: `frontend/src/app/settings/page.tsx`
- Modify: `frontend/src/app/_components/SettingsModal.tsx`, `frontend/src/hooks/useModalState.ts` only if routing a modal to settings

- [ ] **Step 1: Sub-nav**

Replace `Tabs` + animated underline with a two-column layout (remove `tabRefs`, `underlineStyle`, `useLayoutEffect`, `motion` import):
```tsx
    <div className="flex h-full">
      <nav aria-label="Settings sections" className="w-48 shrink-0 space-y-0.5 border-r border-border p-3">
        <h1 className="px-2 pb-3 text-sm font-semibold">Settings</h1>
        {tabs.map(({ value, label, icon: Icon }) => (
          <button
            key={value}
            onClick={() => setActiveTab(value)}
            aria-current={activeTab === value ? 'page' : undefined}
            className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm ${
              activeTab === value ? 'bg-muted text-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'
            }`}
          >
            <Icon className="h-4 w-4" /> {label}
          </button>
        ))}
      </nav>
      <div className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl space-y-6 p-8">
          {activeTab === 'general' && (<><OrgAccountSettings /><PreferenceSettings /></>)}
          {activeTab === 'recording' && <RecordingSettings />}
          {activeTab === 'Transcriptionmodels' && (
            <TranscriptSettings transcriptModelConfig={transcriptModelConfig} setTranscriptModelConfig={setTranscriptModelConfig} />
          )}
          {activeTab === 'summaryModels' && <SummaryModelSettings />}
          {activeTab === 'templates' && <TemplateSettings />}
          {activeTab === 'beta' && <BetaSettings />}
          {activeTab === 'appearance' && <AppearanceSettings />}
        </div>
      </div>
    </div>
```
Support deep links: read `?tab=` with `useSearchParams()` for the initial `activeTab` (fall back to `'general'`, and to `'general'` if the tab is filtered out by `transcriptionManaged`). Wrap the page in `<Suspense>` if `next build` requires it for `useSearchParams`. Keep the managed-policy filter. Remove the Back button (the sidebar provides navigation).

- [ ] **Step 2: Modal audit**

In `SettingsModal.tsx`, list each modal (`modelSettings`, `deviceSettings`, `languageSettings`, `modelSelector`, `errorAlert`, `chunkDropWarning`). Keep `errorAlert`, `chunkDropWarning` and `languageSettings` (it's a quick picker used from the transcript header) as dialogs, restyled via tokens. For `modelSettings`/`modelSelector`/`deviceSettings`: if the modal content is the same component as a settings tab (`TranscriptSettings`, `RecordingSettings`/`DeviceSelection`), replace `showModal(...)` call sites with `router.push('/settings?tab=Transcriptionmodels')` / `'/settings?tab=recording'`, **except** where the modal is opened mid-recording or mid-start-flow (e.g. from `useRecordingStart` when no model is downloaded). Those stay modals so the flow isn't interrupted. Record the final list in the commit message.

- [ ] **Step 3: Verify**

`npx tsc --noEmit && pnpm lint && pnpm build`; every section renders in both themes; with managed policy active (if testable) Transcription is hidden; `/settings?tab=appearance` opens Appearance; `HomeIdle` "Change" link lands on Recordings if you updated it to `/settings?tab=recording` (do so).

- [ ] **Step 4: Commit**

```bash
git add frontend && git commit -m "feat(ui): settings side navigation with deep links; route model/device modals to settings"
```

---

### Task 7: Onboarding restyle

**Files:**
- Modify: `frontend/src/components/onboarding/OnboardingContainer.tsx`, `OnboardingFlow.tsx`, `steps/*.tsx`, `shared/*` (classes only)

- [ ] **Step 1: Restyle**

Container: `flex min-h-screen items-center justify-center bg-background p-6`; card: `w-full max-w-lg rounded-lg border border-border bg-card p-8`; headings `text-xl font-semibold`; body `text-sm text-muted-foreground`; primary buttons `bg-primary text-primary-foreground`; step indicator dots `bg-muted` / active `bg-primary`; progress bars via `ui/progress` (tokens). No step, text or logic changes.

- [ ] **Step 2: Verify**

`npx tsc --noEmit && pnpm lint && pnpm build`. View onboarding in both themes (if there's no reset command, temporarily force `setShowOnboarding(true)` locally in `layout.tsx`, then revert before committing).

- [ ] **Step 3: Commit**

```bash
git add frontend && git commit -m "feat(ui): onboarding in the new style"
```

---

## Self-review notes

- Spec §1 → Task 1; §2 → Task 2; §3 → Task 3 (+ bar in Task 4 per revised spec); §4.1–4.2 → Task 4; §4.3 → Task 5; §4.4 → Task 6; §4.5 → Task 7; §5 verification is in each task; the smoke test is in Task 4.
- Rewrites (Sidebar, page JSX, panels) give the target markup and the list of behaviours to keep rather than a full file copy, because the existing files (882 / 259 lines) must be read and preserved, not replaced from the plan.
