// Asserts WCAG contrast for the palette in docs/superpowers/specs/2026-09-24-ui-redesign-design.md.
const lum = (hex) => {
  const c = [0, 2, 4].map((i) => parseInt(hex.slice(1).slice(i, i + 2), 16) / 255)
    .map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const dark = { bg: '#0e0f12', fg: '#e6e7ea', card: '#131418', muted: '#1d1f25', mfg: '#8a8d97', input: '#5c5f6a', primary: '#8b8cf7', pfg: '#0e0f12', destr: '#f2555a', dfg: '#0e0f12', success: '#4fd1a5',
  speakers: ['#8b8cf7', '#4fd1a5', '#f5a524', '#f47fb5', '#5cc8ff', '#c39cff'] };
const light = { bg: '#fbfbfc', fg: '#16171a', card: '#f4f4f6', muted: '#ececf0', mfg: '#5f6270', input: '#8e919b', primary: '#5b5bd6', pfg: '#ffffff', destr: '#ca2b30', dfg: '#ffffff', success: '#18794e',
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
