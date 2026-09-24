'use client';
import { useRouter } from 'next/navigation';
import { FileText, Mic, Monitor, Moon, Settings, Sun, Upload } from 'lucide-react';
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandShortcut } from '@/components/ui/command';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { useConfig } from '@/contexts/ConfigContext';
import { useTheme } from '@/hooks/useTheme';

export function CommandPalette({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const router = useRouter();
  const { meetings, handleRecordingToggle, setCurrentMeeting } = useSidebar();
  const { isRecording } = useRecordingState();
  const { openImportDialog } = useImportDialog();
  const { betaFeatures } = useConfig();
  const { setTheme } = useTheme();
  const run = (fn: () => void) => () => { onOpenChange(false); fn(); };

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange}>
      <CommandInput placeholder="Type a command or meeting name…" />
      <CommandList>
        <CommandEmpty>No results.</CommandEmpty>
        <CommandGroup heading="Actions">
          <CommandItem onSelect={run(() => (isRecording ? router.push('/') : handleRecordingToggle()))}>
            <Mic /> {isRecording ? 'Go to recording' : 'New recording'} <CommandShortcut>⌘R</CommandShortcut>
          </CommandItem>
          {betaFeatures.importAndRetranscribe && (
            <CommandItem onSelect={run(() => openImportDialog())}><Upload /> Import audio</CommandItem>
          )}
          <CommandItem onSelect={run(() => router.push('/settings'))}>
            <Settings /> Settings <CommandShortcut>⌘,</CommandShortcut>
          </CommandItem>
        </CommandGroup>
        <CommandGroup heading="Theme">
          <CommandItem onSelect={run(() => setTheme('system'))}><Monitor /> System theme</CommandItem>
          <CommandItem onSelect={run(() => setTheme('dark'))}><Moon /> Dark theme</CommandItem>
          <CommandItem onSelect={run(() => setTheme('light'))}><Sun /> Light theme</CommandItem>
        </CommandGroup>
        <CommandGroup heading="Meetings">
          {meetings.map((m) => (
            <CommandItem
              key={m.id}
              value={`${m.title} ${m.id}`}
              onSelect={run(() => {
                // Same as a sidebar row click: keeps the current-meeting highlight in sync.
                setCurrentMeeting({ id: m.id, title: m.title });
                router.push(`/meeting-details?id=${m.id}`);
              })}
            >
              <FileText /> {m.title}
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
