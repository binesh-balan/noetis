interface StatusOverlaysProps {
  // Status flags
  isProcessing: boolean;      // Processing transcription after recording stops
  isSaving: boolean;          // Saving transcript to database
}

// Bottom strip matching RecordingBar, shown while the stopped recording is finalized.
export function StatusOverlays({
  isProcessing,
  isSaving,
}: StatusOverlaysProps) {
  const message = isSaving ? 'Saving…' : isProcessing ? 'Processing transcript…' : null;
  if (!message) return null;

  return (
    <div className="shrink-0 border-t border-border bg-card px-4 py-2" role="status">
      <div className="mx-auto flex max-w-5xl items-center justify-center gap-2">
        <div className="h-4 w-4 animate-spin rounded-full border-b-2 border-primary" />
        <span className="text-sm text-muted-foreground">{message}</span>
      </div>
    </div>
  );
}
