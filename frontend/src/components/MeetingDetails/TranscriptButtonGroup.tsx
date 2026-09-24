"use client";

import { useState, useCallback } from 'react';
import { toast } from 'sonner';
import { identifySpeakers } from '@/lib/speakers';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, FolderOpen, RefreshCw, Users } from 'lucide-react';
import Analytics from '@/lib/analytics';
import { RetranscribeDialog } from './RetranscribeDialog';
import { useConfig } from '@/contexts/ConfigContext';
import { cn } from '@/lib/utils';

/** Shared toolbar button look for the meeting details action rows. */
export const toolbarButtonClass =
  'h-7 gap-1.5 rounded-md border border-border px-2.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground [&_svg]:size-3.5';
/** The one primary action per toolbar (e.g. Generate summary). */
export const toolbarButtonPrimaryClass =
  'bg-primary text-primary-foreground border-transparent hover:bg-primary/90 hover:text-primary-foreground';


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
    try {
      const speakers = await identifySpeakers(meetingId, meetingFolderPath, (message, percent) =>
        toast.loading(`${message} ${percent}%`, { id: toastId }));
      toast.success(`Found ${speakers} speaker${speakers === 1 ? '' : 's'}. Click a name to rename it.`, { id: toastId });
      await onRefetchTranscripts?.();
    } catch (e) {
      toast.error(`Speaker identification failed: ${e instanceof Error ? e.message : e}`, { id: toastId });
    } finally {
      setIdentifying(false);
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
          variant="ghost"
          size="sm"
          className={cn(toolbarButtonClass)}
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
          variant="ghost"
          className={cn(toolbarButtonClass)}
          onClick={() => {
            Analytics.trackButtonClick('open_recording_folder', 'meeting_details');
            onOpenMeetingFolder();
          }}
          title="Open Recording Folder"
        >
          <FolderOpen />
          <span className="hidden @[22rem]:inline">Recording</span>
        </Button>

        {meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant="ghost"
            className={cn(toolbarButtonClass)}
            onClick={() => {
              Analytics.trackButtonClick('identify_speakers', 'meeting_details');
              handleIdentifySpeakers();
            }}
            disabled={identifying || transcriptCount === 0}
            title="Identify who said what (runs on this device)"
          >
            <Users />
            <span className="hidden @[22rem]:inline">Speakers</span>
          </Button>
        )}

        {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant="ghost"
            className={cn(toolbarButtonClass, 'border-primary text-primary hover:bg-primary/10 hover:text-primary')}
            onClick={() => {
              Analytics.trackButtonClick('enhance_transcript', 'meeting_details');
              setShowRetranscribeDialog(true);
            }}
            title="Retranscribe to enhance your recorded audio"
          >
            <RefreshCw />
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
