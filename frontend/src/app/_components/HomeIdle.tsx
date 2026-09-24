'use client';
import type { ReactNode } from 'react';
import Link from 'next/link';
import { Mic, Upload } from 'lucide-react';
import { useConfig } from '@/contexts/ConfigContext';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { PermissionWarning } from '@/components/PermissionWarning';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
import { useIsLinux, useModKeyLabel } from '@/hooks/usePlatform';

/**
 * Idle Home. `startControl` is RecordingControls' own start button (it owns the
 * start flow's device-error handling), so start behaviour is unchanged. When it
 * is null (no microphone permission) a disabled placeholder is shown instead.
 */
export function HomeIdle({ startControl }: { startControl: ReactNode }) {
  const { selectedDevices, transcriptModelConfig, betaFeatures } = useConfig();
  const { openImportDialog } = useImportDialog();
  const { checkPermissions, isChecking, hasSystemAudio, hasMicrophone } = usePermissionCheck();
  const isLinux = useIsLinux();
  const mod = useModKeyLabel();
  const mic = selectedDevices?.micDevice ?? 'Default microphone';
  const system = selectedDevices?.systemDevice ?? 'Default system audio';
  return (
    <div className="flex h-full flex-col items-center justify-center gap-5 overflow-y-auto p-8 text-center">
      {!isChecking && !isLinux && (
        <PermissionWarning
          hasMicrophone={hasMicrophone}
          hasSystemAudio={hasSystemAudio}
          onRecheck={checkPermissions}
          isRechecking={isChecking}
        />
      )}
      {startControl ?? (
        <button
          disabled
          aria-label="Start recording"
          className="flex h-16 w-16 items-center justify-center rounded-full border border-recording/60 bg-card disabled:opacity-50"
        >
          <span className="h-6 w-6 rounded-full bg-recording" />
        </button>
      )}
      <div>
        <h1 className="text-lg font-semibold">Start recording</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          <kbd className="rounded border border-border px-1 text-[11px]">{mod}R</kbd> from anywhere
        </p>
      </div>
      {betaFeatures.importAndRetranscribe && (
        <button onClick={() => openImportDialog()} className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground">
          <Upload className="h-4 w-4" /> Import audio file
        </button>
      )}
      <p className="max-w-md text-xs text-muted-foreground">
        <Mic className="mr-1 inline h-3 w-3" />
        {mic} · {system} · {transcriptModelConfig.model}{' '}
        <Link href="/settings" className="text-primary hover:underline">Change</Link>
      </p>
    </div>
  );
}
