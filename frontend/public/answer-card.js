// Floating answer card (opened by src-tauri/src/live_answers). Polls its state from Rust.
const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
const $ = (id) => document.getElementById(id);
const stored = (key) => { try { return localStorage.getItem(key); } catch { return null; } };

const theme = stored('noetis-theme');
const dark = theme === 'dark' || (theme !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
document.documentElement.classList.toggle('dark', dark);

let last = '';
function render(card) {
  const ready = card.state === 'ready';
  $('thinking').hidden = card.state !== 'thinking';
  $('error').hidden = card.state !== 'error';
  $('error').textContent = card.message || '';
  $('question').hidden = !ready || !card.draft.question;
  $('answer').hidden = !ready;
  $('copy').hidden = !ready;
  $('unsure').hidden = !ready || card.draft.confident;
  if (ready) {
    $('question').textContent = card.draft.question;
    $('answer').textContent = card.draft.answer;
  }
}
// ponytail: polls every 400 ms (local IPC, card open only for a minute); switch to an event if it ever matters.
async function tick() {
  try {
    const card = await invoke('answer_card_state');
    const key = JSON.stringify(card);
    if (key !== last) { last = key; render(card); }
  } catch { /* window closing */ }
}
setInterval(tick, 400);
tick();

$('copy').onclick = () =>
  navigator.clipboard.writeText($('answer').textContent).then(() => {
    $('copy').textContent = 'Copied';
    setTimeout(() => { $('copy').textContent = 'Copy'; }, 1200);
  });
$('dismiss').onclick = () => invoke('answer_card_dismiss');
