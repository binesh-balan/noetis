'use client';

import React, { useState, useMemo, useEffect, useCallback } from 'react';
import Link from 'next/link';
import { FileText, Info, Mic, NotebookPen, PanelLeft, Pencil, Search, Settings, Trash2, Upload, X } from 'lucide-react';
import { useRouter, usePathname } from 'next/navigation';
import { useSidebar } from './SidebarProvider';
import type { CurrentMeeting } from '@/components/Sidebar/SidebarProvider';
import { ConfirmationModal } from '../ConfirmationModel/confirmation-modal';
import Analytics from '@/lib/analytics';
import { invoke } from '@tauri-apps/api/core';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { toast } from 'sonner';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { useConfig } from '@/contexts/ConfigContext';
import { cn } from '@/lib/utils';
import { formatDuration } from '@/lib/formatDuration';
import { meetingHref } from '@/lib/meetingHref';
import { useModKeyLabel } from '@/hooks/usePlatform';
import { useLiveSeconds } from '@/hooks/useLiveSeconds';
import { Input } from '@/components/ui/input';
import { About } from '../About';

import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog"
import { VisuallyHidden } from "@/components/ui/visually-hidden"

// Collapsed-strip icon button with a right-side tooltip.
function RailButton({ label, onClick, active, children }: {
  label: string; onClick: () => void; active?: boolean; children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          onClick={onClick}
          aria-label={label}
          className={cn(
            'flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground',
            active && 'bg-muted text-foreground'
          )}
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent side="right"><p>{label}</p></TooltipContent>
    </Tooltip>
  );
}

const Sidebar: React.FC = () => {
  const router = useRouter();
  const pathname = usePathname();
  const {
    currentMeeting,
    setCurrentMeeting,
    isCollapsed,
    toggleCollapse,
    handleRecordingToggle,
    searchTranscripts,
    searchResults,
    isSearching,
    meetings,
    setMeetings,
  } = useSidebar();

  // Get recording state from RecordingStateContext (single source of truth)
  const { isRecording, isPaused, activeDuration } = useRecordingState();
  const modKey = useModKeyLabel();
  const liveSeconds = useLiveSeconds(activeDuration, isRecording && !isPaused);
  const { openImportDialog } = useImportDialog();
  const { betaFeatures } = useConfig();
  const [searchQuery, setSearchQuery] = useState<string>('');

  // State for edit modal
  const [editModalState, setEditModalState] = useState<{ isOpen: boolean; meetingId: string | null; currentTitle: string }>({
    isOpen: false,
    meetingId: null,
    currentTitle: ''
  });
  const [editingTitle, setEditingTitle] = useState<string>('');

  const [deleteModalState, setDeleteModalState] = useState<{ isOpen: boolean; itemId: string | null }>({ isOpen: false, itemId: null });

  // Handle search input changes
  const handleSearchChange = useCallback(async (value: string) => {
    setSearchQuery(value);

    // If search query is empty, just return to normal view
    if (!value.trim()) return;

    // Search through transcripts
    await searchTranscripts(value);
  }, [searchTranscripts]);

  // Meetings matching the transcript search results or the title
  const filteredMeetings = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return meetings;
    const matchedMeetingIds = new Set(searchResults.map(result => result.id));
    return meetings.filter(m => matchedMeetingIds.has(m.id) || m.title.toLowerCase().includes(q));
  }, [meetings, searchQuery, searchResults]);

  const handleDelete = async (itemId: string) => {
    console.log('Deleting item:', itemId);

    try {
      await invoke('api_delete_meeting', {
        meetingId: itemId,
      });
      console.log('Meeting deleted successfully');
      const updatedMeetings = meetings.filter((m: CurrentMeeting) => m.id !== itemId);
      setMeetings(updatedMeetings);

      // Track meeting deletion
      Analytics.trackMeetingDeleted(itemId);

      // Show success toast
      toast.success("Meeting deleted successfully", {
        description: "All associated data has been removed"
      });

      // If deleting the active meeting, navigate to home
      if (currentMeeting?.id === itemId) {
        setCurrentMeeting({ id: 'intro-call', title: '+ New Call' });
        router.push('/');
      }
    } catch (error) {
      console.error('Failed to delete meeting:', error);
      toast.error("Failed to delete meeting", {
        description: error instanceof Error ? error.message : String(error)
      });
    }
  };

  const handleDeleteConfirm = () => {
    if (deleteModalState.itemId) {
      handleDelete(deleteModalState.itemId);
    }
    setDeleteModalState({ isOpen: false, itemId: null });
  };

  // Handle modal editing of meeting names
  const handleEditStart = (meetingId: string, currentTitle: string) => {
    setEditModalState({
      isOpen: true,
      meetingId: meetingId,
      currentTitle: currentTitle
    });
    setEditingTitle(currentTitle);
  };

  const handleEditConfirm = async () => {
    const newTitle = editingTitle.trim();
    const meetingId = editModalState.meetingId;

    if (!meetingId) return;

    // Prevent empty titles
    if (!newTitle) {
      toast.error("Meeting title cannot be empty");
      return;
    }

    try {
      await invoke('api_save_meeting_title', {
        meetingId: meetingId,
        title: newTitle,
      });

      // Update local state
      const updatedMeetings = meetings.map((m: CurrentMeeting) =>
        m.id === meetingId ? { ...m, title: newTitle } : m
      );
      setMeetings(updatedMeetings);

      // Update current meeting if it's the one being edited
      if (currentMeeting?.id === meetingId) {
        setCurrentMeeting({ id: meetingId, title: newTitle });
      }

      // Track the edit
      Analytics.trackButtonClick('edit_meeting_title', 'sidebar');

      toast.success("Meeting title updated successfully");

      // Close modal and reset state
      setEditModalState({ isOpen: false, meetingId: null, currentTitle: '' });
      setEditingTitle('');
    } catch (error) {
      console.error('Failed to update meeting title:', error);
      toast.error("Failed to update meeting title", {
        description: error instanceof Error ? error.message : String(error)
      });
    }
  };

  const handleEditCancel = () => {
    setEditModalState({ isOpen: false, meetingId: null, currentTitle: '' });
    setEditingTitle('');
  };

  // Exposed on window for the Rust tray. It used to open a model-settings state that
  // no longer rendered anything; it now opens the Settings page.
  useEffect(() => {
    (window as any).openSettings = () => router.push('/settings');

    // Cleanup on unmount
    return () => {
      delete (window as any).openSettings;
    };
  }, [router]);

  const openMeeting = (item: CurrentMeeting) => {
    setCurrentMeeting({ id: item.id, title: item.title });
    router.push(meetingHref(item.id));
  };

  // Find matching transcript snippet for a meeting item
  const findMatchingSnippet = (itemId: string) => {
    if (!searchQuery.trim() || !searchResults.length) return null;
    return searchResults.find(result => result.id === itemId);
  };

  const isSettingsPage = pathname === '/settings';
  const recordingLabel = isPaused ? 'Paused' : 'Recording';
  const recordingDot = (
    <span className={cn('h-2 w-2 shrink-0 rounded-full bg-recording', !isPaused && 'animate-pulse')} />
  );

  const renderMeeting = (item: CurrentMeeting) => {
    const isActive = currentMeeting?.id === item.id;
    const isMeetingItem = item.id.includes('-') && !item.id.startsWith('intro-call');
    const matchingResult = isMeetingItem ? findMatchingSnippet(item.id) : null;

    return (
      <div key={item.id}>
        <div
          className={cn(
            'group flex items-center rounded-md px-2 py-1.5 text-sm text-muted-foreground hover:bg-muted hover:text-foreground',
            isActive && 'bg-muted text-foreground'
          )}
        >
          <button
            onClick={() => openMeeting(item)}
            className="flex min-w-0 flex-1 items-center gap-2 text-left"
          >
            <FileText className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">{item.title}</span>
          </button>
          {isMeetingItem && (
            <div className="flex shrink-0 items-center opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
              <button
                onClick={() => handleEditStart(item.id, item.title)}
                className="rounded p-1 hover:text-foreground"
                aria-label="Edit meeting title"
              >
                <Pencil className="h-3.5 w-3.5" />
              </button>
              <button
                onClick={() => setDeleteModalState({ isOpen: true, itemId: item.id })}
                className="rounded p-1 hover:text-destructive"
                aria-label="Delete meeting"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </div>
          )}
        </div>

        {/* Show transcript match snippet if available */}
        {matchingResult && (
          <div className="mx-2 mb-1 line-clamp-2 border-l-2 border-border pl-2 text-xs text-muted-foreground">
            {matchingResult.matchContext}
          </div>
        )}
      </div>
    );
  };

  const collapseButton = (
    <button
      onClick={toggleCollapse}
      aria-label={isCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
      className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
    >
      <PanelLeft className="h-4 w-4" />
    </button>
  );

  return (
    <aside className={cn('flex h-full shrink-0 flex-col border-r border-border bg-sidebar transition-[width] duration-200', isCollapsed ? 'w-12' : 'w-60')}>
      {isCollapsed ? (
        <div className="flex flex-1 flex-col items-center gap-1 py-2">
          {collapseButton}
          <div className="mt-2 flex flex-col items-center gap-1">
            {isRecording ? (
              <RailButton label={`${recordingLabel} ${formatDuration(liveSeconds)}`} onClick={() => router.push('/')}>
                {recordingDot}
              </RailButton>
            ) : (
              <RailButton label="New recording" onClick={handleRecordingToggle}>
                <Mic className="h-4 w-4 text-recording" />
              </RailButton>
            )}
            {betaFeatures.importAndRetranscribe && (
              <RailButton label="Import audio" onClick={() => openImportDialog()}>
                <Upload className="h-4 w-4" />
              </RailButton>
            )}
            <RailButton label="Search" onClick={toggleCollapse}>
              <Search className="h-4 w-4" />
            </RailButton>
            <RailButton label="Meeting notes" onClick={toggleCollapse} active={pathname?.includes('/meeting-details')}>
              <NotebookPen className="h-4 w-4" />
            </RailButton>
          </div>
          <div className="mt-auto">
            <RailButton label="Settings" onClick={() => router.push('/settings')} active={isSettingsPage}>
              <Settings className="h-4 w-4" />
            </RailButton>
          </div>
        </div>
      ) : (
        <>
          {/* Header */}
          <div className="flex h-11 shrink-0 items-center gap-2 px-3">
            <button onClick={() => router.push('/')} className="text-sm font-semibold hover:text-foreground/80">
              Noetis
            </button>
            <kbd className="rounded border border-border px-1 text-[10px] text-muted-foreground">{modKey}K</kbd>
            <div className="ml-auto">{collapseButton}</div>
          </div>

          {/* Primary actions */}
          <div className="shrink-0 space-y-0.5 px-2">
            {isRecording ? (
              <Link
                href="/"
                className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-foreground hover:bg-muted"
              >
                {recordingDot}
                <span>{recordingLabel}</span>
                {pathname !== '/' && (
                  // On home the RecordingBar already shows the timer.
                  <span className="ml-auto tabular-nums text-muted-foreground">{formatDuration(liveSeconds)}</span>
                )}
              </Link>
            ) : (
              <button
                onClick={handleRecordingToggle}
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm text-foreground hover:bg-muted"
              >
                <Mic className="h-4 w-4 text-recording" />
                <span>New recording</span>
              </button>
            )}
            {betaFeatures.importAndRetranscribe && (
              <button
                onClick={() => openImportDialog()}
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <Upload className="h-4 w-4" />
                <span>Import audio</span>
              </button>
            )}
          </div>

          {/* Transcript search */}
          <div className="relative shrink-0 px-2 pt-3">
            <Search className="pointer-events-none absolute left-4 top-1/2 mt-1.5 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              placeholder="Search meeting content..."
              value={searchQuery}
              onChange={(e) => handleSearchChange(e.target.value)}
              className="h-8 border-input bg-background pl-7 pr-7 text-sm"
            />
            {searchQuery && (
              <button
                onClick={() => handleSearchChange('')}
                aria-label="Clear search"
                className="absolute right-3 top-1/2 mt-1.5 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>

          {/* Meetings. ponytail: meetings carry no date field, so one ungrouped list;
              group by Today/Yesterday/This week/Earlier once api_get_meetings returns a timestamp. */}
          <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-2 pb-2">
            <div className="flex items-center gap-1.5 px-2 pb-1 pt-3 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
              <NotebookPen className="h-3 w-3" />
              <span>Meetings</span>
              {searchQuery && isSearching && (
                <span className="ml-auto animate-pulse normal-case tracking-normal">Searching...</span>
              )}
            </div>
            {filteredMeetings.map(renderMeeting)}
          </div>

          {/* Footer */}
          <div className="flex shrink-0 items-center gap-1 border-t border-border p-2">
            <button
              onClick={() => router.push('/settings')}
              className={cn(
                'flex flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground hover:bg-muted hover:text-foreground',
                isSettingsPage && 'bg-muted text-foreground'
              )}
            >
              <Settings className="h-4 w-4" />
              <span>Settings</span>
              <kbd className="ml-auto rounded border border-border px-1 text-[10px] text-muted-foreground">{modKey},</kbd>
            </button>
            <Dialog aria-describedby={undefined}>
              <DialogTrigger asChild>
                <button
                  aria-label="About Noetis"
                  title="About Noetis"
                  className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
                >
                  <Info className="h-4 w-4" />
                </button>
              </DialogTrigger>
              <DialogContent>
                <VisuallyHidden>
                  <DialogTitle>About Noetis</DialogTitle>
                </VisuallyHidden>
                <About />
              </DialogContent>
            </Dialog>
          </div>
        </>
      )}

      {/* Confirmation Modal for Delete */}
      <ConfirmationModal
        isOpen={deleteModalState.isOpen}
        text="Are you sure you want to delete this meeting? This action cannot be undone."
        onConfirm={handleDeleteConfirm}
        onCancel={() => setDeleteModalState({ isOpen: false, itemId: null })}
      />

      {/* Edit Meeting Title Modal */}
      <Dialog open={editModalState.isOpen} onOpenChange={(open) => {
        if (!open) handleEditCancel();
      }}>
        <DialogContent className="sm:max-w-[425px]">
          <VisuallyHidden>
            <DialogTitle>Edit Meeting Title</DialogTitle>
          </VisuallyHidden>
          <div className="py-4">
            <h3 className="text-lg font-semibold mb-4">Edit Meeting Title</h3>
            <div className="space-y-4">
              <div>
                <label htmlFor="meeting-title" className="block text-sm font-medium text-foreground mb-2">
                  Meeting Title
                </label>
                <input
                  id="meeting-title"
                  type="text"
                  value={editingTitle}
                  onChange={(e) => setEditingTitle(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      handleEditConfirm();
                    } else if (e.key === 'Escape') {
                      handleEditCancel();
                    }
                  }}
                  className="w-full px-3 py-2 border border-input rounded-md focus:outline-none focus:ring-2 focus:ring-primary focus:border-transparent"
                  placeholder="Enter meeting title"
                  autoFocus
                />
              </div>
            </div>
          </div>
          <DialogFooter>
            <button
              onClick={handleEditCancel}
              className="px-4 py-2 text-sm font-medium text-foreground bg-muted hover:bg-accent rounded-md transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={handleEditConfirm}
              className="px-4 py-2 text-sm font-medium text-primary-foreground bg-primary hover:bg-primary/90 rounded-md transition-colors"
            >
              Save
            </button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </aside>
  );
};

export default Sidebar;
