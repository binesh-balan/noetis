'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';

export interface Profile {
  about: string;
  names: string;
}

export function AboutMeSettings() {
  const [profile, setProfile] = useState<Profile | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    invoke<Profile>('get_profile')
      .then(setProfile)
      .catch((e) => {
        toast.error('Failed to load About me', { description: String(e) });
        setProfile({ about: '', names: '' });
      });
  }, []);

  if (!profile) return <div className="h-8 animate-pulse rounded bg-accent" />;

  const save = async () => {
    setSaving(true);
    try {
      await invoke('set_profile', { profile });
      toast.success('About me saved');
    } catch (e) {
      toast.error('Save failed', { description: String(e) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="mb-2 text-lg font-semibold">About me</h3>
        <p className="text-sm text-muted-foreground">
          Noetis uses this wherever it writes for you: meeting summaries and live answer suggestions.
          It is stored only on this computer and sent only to your summary model, with each request.
        </p>
      </div>
      <div className="space-y-2">
        <Label htmlFor="profile-names">Your name and nicknames</Label>
        <Input
          id="profile-names"
          value={profile.names}
          maxLength={500}
          placeholder="Alex, AJ"
          onChange={(e) => setProfile({ ...profile, names: e.target.value })}
        />
        <p className="text-xs text-muted-foreground">Comma-separated. Live answers listen for these.</p>
      </div>
      <div className="space-y-2">
        <Label htmlFor="profile-about">About you</Label>
        <Textarea
          id="profile-about"
          rows={8}
          maxLength={2000}
          value={profile.about}
          placeholder="Your role and team, what you're responsible for, current projects, and how you like to come across."
          onChange={(e) => setProfile({ ...profile, about: e.target.value })}
        />
        <p className="text-right text-xs text-muted-foreground">{profile.about.length}/2000</p>
      </div>
      <Button onClick={save} disabled={saving}>{saving ? 'Saving…' : 'Save'}</Button>
    </div>
  );
}
