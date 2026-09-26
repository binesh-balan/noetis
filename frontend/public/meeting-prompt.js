// Meeting prompt window (opened by the Rust meeting detector).
const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
const stored = (key) => { try { return localStorage.getItem(key); } catch { return null; } };

const theme = stored('noetis-theme');
const dark = theme === 'dark' || (theme !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
document.documentElement.classList.toggle('dark', dark);
if (stored('isAutoSummary') !== 'true') document.getElementById('hint').hidden = false;

invoke('meeting_prompt_info').then((app) => {
  document.getElementById('title').textContent = `🎙 ${app || 'Meeting'} meeting detected`;
});
document.getElementById('record').onclick = () => invoke('meeting_prompt_respond', { record: true });
document.getElementById('dismiss').onclick = () => invoke('meeting_prompt_respond', { record: false });
