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
