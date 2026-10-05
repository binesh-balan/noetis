// Fails when raw Tailwind palette classes are used instead of theme tokens.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

const RAW = /\b(?:[a-z-]+:)*(?:bg|text|border(?:-[trblxy])?|ring|fill|stroke|from|to|via|divide(?:-[xy])?|outline|placeholder|decoration|accent|caret|shadow)-(?:gray|slate|zinc|neutral|stone|blue|sky|indigo|red|rose|green|emerald|yellow|amber|orange|purple|violet|fuchsia|pink|teal|cyan|lime|white|black)(?:-\d{2,3})?(?:\/\d+)?\b/g;
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
