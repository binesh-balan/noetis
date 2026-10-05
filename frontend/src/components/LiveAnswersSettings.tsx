'use client';

import { useEffect, useState } from 'react';
import Link from 'next/link';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import type { Profile } from './AboutMeSettings';

interface LiveAnswerSettings {
  enabled: boolean;
  hotkey: string;
}

export function LiveAnswersSettings() {
  const [settings, setSettings] = useState<LiveAnswerSettings | null>(null);
  const [hotkey, setHotkey] = useState('');
  const [names, setNames] = useState<string | null>(null);

  useEffect(() => {
    invoke<LiveAnswerSettings>('live_answers_get_settings')
      .then((s) => { setSettings(s); setHotkey(s.hotkey); })
      .catch((e) => toast.error('Failed to load live answers', { description: String(e) }));
    invoke<Profile>('get_profile').then((p) => setNames(p.names.trim())).catch(() => setNames(''));
  }, []);

  if (!settings) return <div className="h-8 animate-pulse rounded bg-accent" />;

  const save = async (next: LiveAnswerSettings) => {
    try {
      await invoke('live_answers_set_settings', { settings: next });
      setSettings(next);
      setHotkey(next.hotkey);
    } catch (e) {
      toast.error("Couldn't save live answers", { description: String(e) });
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="mb-2 text-lg font-semibold">Live answers</h3>
        <p className="text-sm text-muted-foreground">
          While recording, when someone asks you a question by name, or you press the hotkey, Noetis drafts a
          short answer in a small card that only you can see (it is hidden from screen sharing). You say it
          in your own words; Noetis never speaks for you.
        </p>
      </div>
      <div className="flex items-center justify-between gap-4 rounded-lg border p-4">
        <Label htmlFor="live-answers-enabled" className="font-medium">Suggest answers during meetings</Label>
        <Switch id="live-answers-enabled" checked={settings.enabled} onCheckedChange={(enabled) => save({ ...settings, enabled })} />
      </div>
      {names === '' && (
        <p className="text-sm text-warning">
          Add your name under <Link href="/settings?tab=aboutMe" className="underline">About me</Link> so Noetis
          knows when you're asked something. Until then only the hotkey works.
        </p>
      )}
      <div className="space-y-2">
        <Label htmlFor="live-answers-hotkey">Hotkey</Label>
        <div className="flex gap-2">
          <Input id="live-answers-hotkey" value={hotkey} onChange={(e) => setHotkey(e.target.value)} placeholder="CommandOrControl+Shift+Space" />
          <Button variant="outline" disabled={hotkey.trim() === settings.hotkey} onClick={() => save({ ...settings, hotkey: hotkey.trim() })}>
            Save
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">Works while Teams or Zoom is in front. Example: CommandOrControl+Shift+Space.</p>
      </div>
      <p className="text-xs text-muted-foreground">
        Each suggestion sends the last 5 minutes of transcript, About me and this meeting's context to your
        summary model (Settings → Summary). Suggestions are not saved. In Strict Offline Mode this works only
        with a local summary model.
      </p>
    </div>
  );
}
