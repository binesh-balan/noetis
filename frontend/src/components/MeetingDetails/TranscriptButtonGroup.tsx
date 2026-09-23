"use client";

import { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, FolderOpen, RefreshCw, Users } from 'lucide-react';
import Analytics from '@/lib/analytics';
import { RetranscribeDialog } from './RetranscribeDialog';
import { useConfig } from '@/contexts/ConfigContext';


interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
}


export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
}: TranscriptButtonGroupProps) {
  const { betaFeatures } = useConfig();
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);

  const [identifying, setIdentifying] = useState(false);

  // Runs on-device speaker identification and reports progress in a single toast.
  const handleIdentifySpeakers = useCallback(async () => {
    if (!meetingId || !meetingFolderPath) return;
    const toastId = toast.loading('Identifying speakers...');
    setIdentifying(true);
    const unlisteners: Array<() => void> = [];
    const done = () => { unlisteners.forEach(u => u()); setIdentifying(false); };
    type Payload = { meeting_id: string; message?: string; progress_percentage?: number; speakers?: number; error?: string };
    try {
      unlisteners.push(await listen<Payload>('diarization-progress', e => {
        if (e.payload.meeting_id === meetingId) toast.loading(`${e.payload.message} ${e.payload.progress_percentage}%`, { id: toastId });
      }));
      unlisteners.push(await listen<Payload>('diarization-complete', async e => {
        if (e.payload.meeting_id !== meetingId) return;
        done();
        toast.success(`Found ${e.payload.speakers} speaker${e.payload.speakers === 1 ? '' : 's'}. Click a name to rename it.`, { id: toastId });
        await onRefetchTranscripts?.();
      }));
      unlisteners.push(await listen<Payload>('diarization-error', e => {
        if (e.payload.meeting_id !== meetingId) return;
        done();
        toast.error(`Speaker identification failed: ${e.payload.error}`, { id: toastId });
      }));
      await invoke('start_speaker_identification', { meetingId, meetingFolderPath });
    } catch (e) {
      done();
      toast.error(String(e), { id: toastId });
    }
  }, [meetingId, meetingFolderPath, onRefetchTranscripts]);

  const handleRetranscribeComplete = useCallback(async () => {
    // Refetch transcripts to show the updated data
    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [onRefetchTranscripts]);

  return (
    <div className="flex items-center justify-center w-full gap-2">
      <ButtonGroup>
        <Button
          variant="outline"
          size="sm"
          className="px-2 @[22rem]:px-3"
          onClick={() => {
            Analytics.trackButtonClick('copy_transcript', 'meeting_details');
            onCopyTranscript();
          }}
          disabled={transcriptCount === 0}
          title={transcriptCount === 0 ? 'No transcript available' : 'Copy Transcript'}
        >
          <Copy />
          <span className="hidden @[22rem]:inline">Copy</span>
        </Button>

        <Button
          size="sm"
          variant="outline"
          className="px-2 @[22rem]:px-4"
          onClick={() => {
            Analytics.trackButtonClick('open_recording_folder', 'meeting_details');
            onOpenMeetingFolder();
          }}
          title="Open Recording Folder"
        >
          <FolderOpen className="@[22rem]:mr-2" size={18} />
          <span className="hidden @[22rem]:inline">Recording</span>
        </Button>

        {meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant="outline"
            className="px-2 @[22rem]:px-4"
            onClick={() => {
              Analytics.trackButtonClick('identify_speakers', 'meeting_details');
              handleIdentifySpeakers();
            }}
            disabled={identifying || transcriptCount === 0}
            title="Identify who said what (runs on this device)"
          >
            <Users className="@[22rem]:mr-2" size={18} />
            <span className="hidden @[22rem]:inline">Speakers</span>
          </Button>
        )}

        {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant="outline"
            className="bg-gradient-to-r from-blue-50 to-purple-50 hover:from-blue-100 hover:to-purple-100 border-blue-200 px-2 @[22rem]:px-4"
            onClick={() => {
              Analytics.trackButtonClick('enhance_transcript', 'meeting_details');
              setShowRetranscribeDialog(true);
            }}
            title="Retranscribe to enhance your recorded audio"
          >
            <RefreshCw className="@[22rem]:mr-2" size={18} />
            <span className="hidden @[22rem]:inline">Enhance</span>
          </Button>
        )}
      </ButtonGroup>

      {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={handleRetranscribeComplete}
        />
      )}
    </div>
  );
}
