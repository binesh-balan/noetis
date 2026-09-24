'use client';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { LANGUAGES } from '@/components/LanguageSelection';

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
      <p className="mt-auto text-xs text-muted-foreground">The summary appears here after you stop.</p>
    </div>
  );
}
