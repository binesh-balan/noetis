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
