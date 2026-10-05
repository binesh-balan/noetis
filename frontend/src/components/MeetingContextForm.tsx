'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';

interface MeetingContext {
  purpose: string;
  with: string;
  topics: string;
}

const EMPTY: MeetingContext = { purpose: '', with: '', topics: '' };

/** The current (or next) recording's context, or a saved meeting's when `meetingId` is set. */
export function MeetingContextForm({ meetingId }: { meetingId?: string }) {
  const [ctx, setCtx] = useState<MeetingContext | null>(null);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    const load = meetingId
      ? invoke<MeetingContext>('get_meeting_context', { meetingId })
      : invoke<MeetingContext>('get_live_meeting_context');
    load.then(setCtx).catch(() => setCtx(EMPTY));
  }, [meetingId]);

  if (!ctx) return null;

  const set = (patch: Partial<MeetingContext>) => {
    setCtx({ ...ctx, ...patch });
    setDirty(true);
  };
  const save = async () => {
    try {
      if (meetingId) await invoke('save_meeting_context', { meetingId, context: ctx });
      else await invoke('set_live_meeting_context', { context: ctx });
      setDirty(false);
      toast.success('Meeting context saved');
    } catch (e) {
      toast.error("Couldn't save meeting context", { description: String(e) });
    }
  };
  const id = meetingId ?? 'live';

  return (
    <div className="space-y-3 text-left">
      <div className="space-y-1">
        <Label htmlFor={`ctx-purpose-${id}`} className="text-xs">What is this meeting about?</Label>
        <Input id={`ctx-purpose-${id}`} maxLength={500} value={ctx.purpose} placeholder="Q4 rollout review" onChange={(e) => set({ purpose: e.target.value })} />
      </div>
      <div className="space-y-1">
        <Label htmlFor={`ctx-with-${id}`} className="text-xs">Who are you meeting?</Label>
        <Input id={`ctx-with-${id}`} maxLength={500} value={ctx.with} placeholder="Contoso: Priya (PM), Marcus (IT lead)" onChange={(e) => set({ with: e.target.value })} />
      </div>
      <div className="space-y-1">
        <Label htmlFor={`ctx-topics-${id}`} className="text-xs">What are you discussing?</Label>
        <Textarea id={`ctx-topics-${id}`} rows={2} maxLength={500} value={ctx.topics} placeholder="Rollout dates, licence count, training plan" onChange={(e) => set({ topics: e.target.value })} />
      </div>
      <Button size="sm" onClick={save} disabled={!dirty}>Save context</Button>
    </div>
  );
}
