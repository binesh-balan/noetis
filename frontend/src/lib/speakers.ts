import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

type Payload = { meeting_id: string; message?: string; progress_percentage?: number; speakers?: number; error?: string };

// In-flight runs by meeting, so the meeting page can wait for a run started elsewhere
// (e.g. automatically after recording) before auto-summarising.
const inFlight = new Map<string, Promise<number>>();

/** Runs on-device speaker identification; resolves with the number of speakers found. */
export function identifySpeakers(
  meetingId: string,
  meetingFolderPath: string,
  onProgress?: (message: string, percent: number) => void,
): Promise<number> {
  const existing = inFlight.get(meetingId);
  if (existing) return existing;

  const run = (async () => {
    const unlisteners: Array<() => void> = [];
    try {
      return await new Promise<number>(async (resolve, reject) => {
        unlisteners.push(await listen<Payload>('diarization-progress', e => {
          if (e.payload.meeting_id === meetingId) onProgress?.(e.payload.message ?? '', e.payload.progress_percentage ?? 0);
        }));
        unlisteners.push(await listen<Payload>('diarization-complete', e => {
          if (e.payload.meeting_id === meetingId) resolve(e.payload.speakers ?? 0);
        }));
        unlisteners.push(await listen<Payload>('diarization-error', e => {
          if (e.payload.meeting_id === meetingId) reject(new Error(e.payload.error));
        }));
        invoke('start_speaker_identification', { meetingId, meetingFolderPath }).catch(reject);
      });
    } finally {
      unlisteners.forEach(u => u());
      inFlight.delete(meetingId);
    }
  })();
  inFlight.set(meetingId, run);
  return run;
}

/** The run in progress for this meeting, if any. */
export function pendingSpeakerIdentification(meetingId: string): Promise<number> | undefined {
  return inFlight.get(meetingId);
}
