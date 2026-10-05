import { useCallback, RefObject } from 'react';
import { MeetingSummary, Transcript } from '@/types';
import { BlockNoteSummaryViewRef } from '@/components/AISummary/BlockNoteSummaryView';
import { toast } from 'sonner';
import Analytics from '@/lib/analytics';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { hasVisibleSummaryContent } from '@/lib/summary-content';
import { markdownToDocx, markdownToPdf, type ExportFormat } from '@/lib/export-document';

interface UseCopyOperationsProps {
  meeting: any;
  transcripts: Transcript[];
  meetingTitle: string;
  aiSummary: MeetingSummary | null;
  blockNoteSummaryRef: RefObject<BlockNoteSummaryViewRef>;
}

const DATE_FORMAT: Intl.DateTimeFormatOptions = {
  year: 'numeric',
  month: 'long',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit'
};

export function useCopyOperations({
  meeting,
  transcripts,
  meetingTitle,
  aiSummary,
  blockNoteSummaryRef,
}: UseCopyOperationsProps) {

  // Helper function to fetch ALL transcripts for copying (not just paginated data)
  const fetchAllTranscripts = useCallback(async (meetingId: string): Promise<Transcript[]> => {
    try {
      console.log('📊 Fetching all transcripts for copying:', meetingId);

      // First, get total count by fetching first page
      const firstPage = await invokeTauri('api_get_meeting_transcripts', {
        meetingId,
        limit: 1,
        offset: 0,
      }) as { transcripts: Transcript[]; total_count: number; has_more: boolean };

      const totalCount = firstPage.total_count;
      console.log(`📊 Total transcripts in database: ${totalCount}`);

      if (totalCount === 0) {
        return [];
      }

      // Fetch all transcripts in one call
      const allData = await invokeTauri('api_get_meeting_transcripts', {
        meetingId,
        limit: totalCount,
        offset: 0,
      }) as { transcripts: Transcript[]; total_count: number; has_more: boolean };

      console.log(`✅ Fetched ${allData.transcripts.length} transcripts from database for copying`);
      return allData.transcripts;
    } catch (error) {
      console.error('❌ Error fetching all transcripts:', error);
      toast.error('Failed to fetch transcripts for copying');
      return [];
    }
  }, []);

  // Full transcript as markdown (all pages, not just the paginated view); null if empty.
  const buildTranscriptMarkdown = useCallback(async () => {
    const allTranscripts = await fetchAllTranscripts(meeting.id);
    if (!allTranscripts.length) return null;

    // Format timestamps as recording-relative [MM:SS] instead of wall-clock time
    const formatTime = (seconds: number | undefined, fallbackTimestamp: string): string => {
      if (seconds === undefined) {
        // For old transcripts without audio_start_time, use wall-clock time
        return fallbackTimestamp;
      }
      const totalSecs = Math.floor(seconds);
      const mins = Math.floor(totalSecs / 60);
      const secs = totalSecs % 60;
      return `[${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}]`;
    };

    const header = `# Transcript of the Meeting: ${meeting.id} - ${meetingTitle ?? meeting.title}\n\n`;
    const date = `## Date: ${new Date(meeting.created_at).toLocaleDateString()}\n\n`;
    const fullTranscript = allTranscripts
      .map(t => `${formatTime(t.audio_start_time, t.timestamp)} ${t.speaker ? `**${t.speaker}:** ` : ''}${t.text}  `)
      .join('\n');

    return { markdown: header + date + fullTranscript, allTranscripts };
  }, [meeting, meetingTitle, fetchAllTranscripts]);

  // Copy transcript to clipboard
  const handleCopyTranscript = useCallback(async () => {
    const built = await buildTranscriptMarkdown();
    if (!built) {
      toast.error('No transcripts available to copy');
      return;
    }
    const { markdown, allTranscripts } = built;

    await navigator.clipboard.writeText(markdown);
    toast.success("Transcript copied to clipboard");

    // Track copy analytics
    const wordCount = allTranscripts
      .map(t => t.text.split(/\s+/).length)
      .reduce((a, b) => a + b, 0);

    await Analytics.trackCopy('transcript', {
      meeting_id: meeting.id,
      transcript_length: allTranscripts.length.toString(),
      word_count: wordCount.toString()
    });
  }, [meeting, buildTranscriptMarkdown]);

  // Summary as markdown with a metadata header; null if there is no summary content.
  const buildSummaryMarkdown = useCallback(async (): Promise<string | null> => {
    if (!hasVisibleSummaryContent(aiSummary)) return null;
    let summaryMarkdown = '';

    // Try to get markdown from BlockNote editor first
    if (blockNoteSummaryRef.current?.getMarkdown) {
      summaryMarkdown = await blockNoteSummaryRef.current.getMarkdown();
    }

    // Fallback: Check if aiSummary has markdown property
    if (!summaryMarkdown && aiSummary && typeof aiSummary.markdown === 'string') {
      summaryMarkdown = aiSummary.markdown;
    }

    // Fallback: Check for legacy format
    if (!summaryMarkdown && aiSummary) {
      summaryMarkdown = Object.entries(aiSummary)
        .filter(([key]) => {
          // Skip non-section keys
          return key !== 'markdown' && key !== 'summary_json' && key !== '_section_order' && key !== 'MeetingName';
        })
        .map(([, section]) => {
          if (section && typeof section === 'object' && 'title' in section && 'blocks' in section) {
            const sectionTitle = `## ${section.title}\n\n`;
            const sectionContent = section.blocks
              .map((block: any) => `- ${block.content}`)
              .join('\n');
            return sectionTitle + sectionContent;
          }
          return '';
        })
        .filter(s => s.trim())
        .join('\n\n');
    }

    if (!summaryMarkdown.trim()) return null;

    // Build metadata header
    const header = `# Meeting Summary: ${meetingTitle}\n\n`;
    const metadata = `**Meeting ID:** ${meeting.id}\n**Date:** ${new Date(meeting.created_at).toLocaleDateString('en-US', DATE_FORMAT)}\n**Generated on:** ${new Date().toLocaleDateString('en-US', DATE_FORMAT)}\n\n---\n\n`;

    return header + metadata + summaryMarkdown;
  }, [aiSummary, meetingTitle, meeting, blockNoteSummaryRef]);

  // Copy summary to clipboard
  const handleCopySummary = useCallback(async () => {
    try {
      const fullMarkdown = await buildSummaryMarkdown();
      if (!fullMarkdown) {
        toast.error('No summary content available to copy');
        return;
      }
      await navigator.clipboard.writeText(fullMarkdown);
      toast.success("Summary copied to clipboard");

      // Track copy analytics
      await Analytics.trackCopy('summary', {
        meeting_id: meeting.id,
        has_markdown: (!!aiSummary && 'markdown' in aiSummary).toString()
      });
    } catch (error) {
      console.error('❌ Failed to copy summary:', error);
      toast.error("Failed to copy summary");
    }
  }, [aiSummary, meeting, buildSummaryMarkdown]);

  // Export summary (+ optional transcript appendix) as PDF / Word / Markdown via a native save dialog.
  const handleExport = useCallback(async (format: ExportFormat, includeTranscript: boolean) => {
    try {
      const summary = await buildSummaryMarkdown();
      const transcript = includeTranscript ? (await buildTranscriptMarkdown())?.markdown : null;
      if (!summary && !transcript) {
        toast.error('Nothing to export yet');
        return;
      }
      const markdown = [summary, transcript].filter(Boolean).join('\n\n---\n\n');
      const suggestedFilename = `${(meetingTitle || 'meeting').replace(/[\\/:*?"<>|]/g, '_')}.${format}`;

      const saved = format === 'md'
        ? await invokeTauri<boolean>('export_text_content', { content: markdown, suggestedFilename, extension: 'md' })
        : await invokeTauri<boolean>('export_binary_content', {
            content: Array.from(format === 'pdf' ? await markdownToPdf(markdown) : await markdownToDocx(markdown)),
            suggestedFilename,
            extension: format,
          });
      if (saved) toast.success(`Exported ${format.toUpperCase()}`);
    } catch (error) {
      console.error('❌ Export failed:', error);
      toast.error(`Export failed: ${error}`);
    }
  }, [meetingTitle, buildSummaryMarkdown, buildTranscriptMarkdown]);

  return {
    handleCopyTranscript,
    handleCopySummary,
    handleExport,
  };
}
