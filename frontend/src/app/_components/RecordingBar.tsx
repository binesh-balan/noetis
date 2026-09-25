'use client';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useLiveSeconds } from '@/hooks/useLiveSeconds';
import { formatDuration } from '@/lib/formatDuration';

export function RecordingBar({ children }: { children: React.ReactNode }) {
  const { isRecording, isPaused, activeDuration } = useRecordingState();
  const seconds = useLiveSeconds(activeDuration, isRecording && !isPaused);
  return (
    <div className="shrink-0 border-t border-border bg-card px-4 py-2">
      <div className="mx-auto flex max-w-5xl items-center justify-center gap-4">
        {isRecording && (
          <span className="flex items-center gap-2 text-sm">
            <span className={`h-2.5 w-2.5 rounded-full ${isPaused ? 'bg-warning' : 'animate-pulse bg-recording'}`} />
            <span className="tabular-nums">{formatDuration(seconds)}</span>
          </span>
        )}
        {children}
      </div>
    </div>
  );
}
