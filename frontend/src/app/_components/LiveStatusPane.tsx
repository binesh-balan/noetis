'use client';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { LANGUAGES } from '@/components/LanguageSelection';
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { MeetingContextForm } from '@/components/MeetingContextForm';

interface Draft {
  question: string;
  answer: string;
  confident: boolean;
}

function AnswerSuggestions() {
  const [items, setItems] = useState<Draft[]>([]);
  useEffect(() => {
    invoke<Draft[]>('live_answer_history').then(setItems).catch(() => undefined);
    const unlisten = listen<Draft>('live-answer', (e) => setItems((prev) => [...prev, e.payload]));
    return () => { unlisten.then((fn) => fn()); };
  }, []);
  if (items.length === 0) {
    return <p className="text-xs text-muted-foreground">Answer suggestions appear here when live answers are on (Settings → Live answers).</p>;
  }
  return (
    <ul className="space-y-3">
      {[...items].reverse().map((d, i) => (
        <li key={items.length - i} className="space-y-1 rounded-md border border-border p-2 text-sm">
          {d.question && <p className="text-xs text-muted-foreground">{d.question}</p>}
          <p>{d.answer}</p>
          {!d.confident && <p className="text-xs text-warning">unsure</p>}
        </li>
      ))}
    </ul>
  );
}

const Row = ({ k, v }: { k: string; v: string }) => (
  <div className="flex items-baseline justify-between gap-3 text-sm">
    <span className="text-muted-foreground">{k}</span>
    <span className="truncate text-right" title={v}>{v}</span>
  </div>
);

export function LiveStatusPane() {
  const { selectedDevices, transcriptModelConfig, selectedLanguage } = useConfig();
  const { isPaused } = useRecordingState();
  const language = LANGUAGES.find((l) => l.code === selectedLanguage)?.name ?? selectedLanguage;
  return (
    <div className="flex h-full flex-col gap-5 overflow-y-auto p-4">
      <section className="space-y-2">
        <h2 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Input</h2>
        <Row k="Microphone" v={selectedDevices?.micDevice ?? 'Default'} />
        <Row k="System" v={selectedDevices?.systemDevice ?? 'Default'} />
        <Row k="State" v={isPaused ? 'Paused' : 'Listening'} />
      </section>
      <section className="space-y-2">
        <h2 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Engine</h2>
        <Row k="Provider" v={transcriptModelConfig.provider} />
        <Row k="Model" v={transcriptModelConfig.model} />
        {language && <Row k="Language" v={language} />}
      </section>
      <section className="space-y-2">
        <h2 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Meeting context</h2>
        <MeetingContextForm />
      </section>
      <section className="space-y-2">
        <h2 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Answer suggestions</h2>
        <AnswerSuggestions />
      </section>
      <p className="mt-auto text-xs text-muted-foreground">The summary appears here after you stop.</p>
    </div>
  );
}
