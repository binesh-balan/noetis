'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';

interface Voice {
  name: string;
  meetings: number;
  last_heard: string;
}

export function SpeakerSettings() {
  const [voices, setVoices] = useState<Voice[] | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const cancelledRef = useRef(false);

  const load = useCallback(async () => {
    try {
      setVoices(await invoke<Voice[]>('api_list_voices'));
    } catch (e) {
      toast.error('Failed to load voices', { description: String(e) });
      setVoices([]);
    }
  }, []);
  useEffect(() => { load(); }, [load]);

  const rename = async (from: string) => {
    if (cancelledRef.current) {
      cancelledRef.current = false;
      return;
    }
    const to = draft.trim();
    setEditing(null);
    if (!to || to === from) return;
    try {
      await invoke('api_rename_voice', { from, to });
      await load();
    } catch (e) {
      toast.error('Rename failed', { description: String(e) });
    }
  };

  const forget = async (name: string) => {
    if (!window.confirm(`Forget ${name}'s voice? Past transcripts keep the name.`)) return;
    try {
      await invoke('api_forget_voice', { name });
      await load();
    } catch (e) {
      toast.error('Forget failed', { description: String(e) });
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-semibold mb-2">Speakers</h3>
        <p className="text-sm text-muted-foreground">
          Rename a speaker in any meeting and Noetis recognises their voice next time.
          Voiceprints are stored only on this computer.
        </p>
      </div>
      {voices === null ? (
        <div className="h-8 animate-pulse rounded bg-accent" />
      ) : voices.length === 0 ? (
        <p className="text-sm text-muted-foreground">No known voices yet.</p>
      ) : (
        <ul className="divide-y rounded-lg border">
          {voices.map((v) => (
            <li key={v.name} className="flex items-center gap-3 p-3">
              <div className="min-w-0 flex-1">
                {editing === v.name ? (
                  <Input
                    autoFocus
                    value={draft}
                    maxLength={64}
                    aria-label={`New name for ${v.name}`}
                    onChange={(e) => setDraft(e.target.value)}
                    onBlur={() => rename(v.name)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') rename(v.name);
                      if (e.key === 'Escape') {
                        cancelledRef.current = true;
                        setEditing(null);
                      }
                    }}
                  />
                ) : (
                  <>
                    <div className="truncate font-medium">{v.name}</div>
                    <div className="text-xs text-muted-foreground">
                      {v.meetings} {v.meetings === 1 ? 'meeting' : 'meetings'} · last heard{' '}
                      {new Date(v.last_heard).toLocaleDateString()}
                    </div>
                  </>
                )}
              </div>
              <Button variant="ghost" size="sm" onClick={() => { cancelledRef.current = false; setEditing(v.name); setDraft(v.name); }}>
                Rename
              </Button>
              <Button variant="ghost" size="sm" className="text-destructive" onClick={() => forget(v.name)}>
                Forget
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
