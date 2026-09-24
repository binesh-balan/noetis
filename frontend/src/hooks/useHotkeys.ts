'use client';
import { useEffect, useRef } from 'react';

/** Keys: 'mod+k', 'mod+r', 'mod+\', 'mod+,'. `mod` = ⌘ on macOS, Ctrl elsewhere.
 *  Only 'mod+k' fires while typing in an input/textarea/contenteditable. */
export function useHotkeys(map: Record<string, (e: KeyboardEvent) => void>) {
  const ref = useRef(map);
  ref.current = map;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
      const key = e.key.toLowerCase();
      const t = e.target as HTMLElement | null;
      const typing = !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
      if (typing && key !== 'k') return;
      const handler = ref.current[`mod+${key}`];
      if (handler) {
        e.preventDefault();
        handler(e);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}
