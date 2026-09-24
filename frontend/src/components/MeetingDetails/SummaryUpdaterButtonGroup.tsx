"use client";

import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, Save, Loader2, Download } from 'lucide-react';
import { useState } from 'react';
import Analytics from '@/lib/analytics';
import {
  DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem,
  DropdownMenuCheckboxItem, DropdownMenuSeparator,
} from '@/components/ui/dropdown-menu';
import type { ExportFormat } from '@/lib/export-document';

interface SummaryUpdaterButtonGroupProps {
  isSaving: boolean;
  isDirty: boolean;
  onSave: () => Promise<void>;
  onCopy: () => Promise<void>;
  onExport: (format: ExportFormat, includeTranscript: boolean) => Promise<void>;
}

export function SummaryUpdaterButtonGroup({
  isSaving,
  isDirty,
  onSave,
  onCopy,
  onExport,
}: SummaryUpdaterButtonGroupProps) {
  const [includeTranscript, setIncludeTranscript] = useState(true);
  const exportAs = (format: ExportFormat) => {
    Analytics.trackButtonClick(`export_${format}`, 'meeting_details');
    onExport(format, includeTranscript);
  };

  return (
    <ButtonGroup>
      {/* Save button */}
      <Button
        variant="outline"
        size="sm"
        className={`${isDirty ? 'bg-success' : ""}`}
        title={isSaving ? "Saving" : "Save Changes"}
        onClick={() => {
          Analytics.trackButtonClick('save_changes', 'meeting_details');
          onSave();
        }}
        disabled={isSaving}
      >
        {isSaving ? (
          <>
            <Loader2 className="animate-spin" />
            <span className="hidden @[40rem]:inline">Saving...</span>
          </>
        ) : (
          <>
            <Save />
            <span className="hidden @[40rem]:inline">Save</span>
          </>
        )}
      </Button>

      {/* Copy button */}
      <Button
        variant="outline"
        size="sm"
        title="Copy Summary"
        onClick={() => {
          Analytics.trackButtonClick('copy_summary', 'meeting_details');
          onCopy();
        }}
        className="cursor-pointer"
      >
        <Copy />
        <span className="hidden @[40rem]:inline">Copy</span>
      </Button>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="outline" size="sm" title="Export" className="cursor-pointer">
            <Download />
            <span className="hidden @[40rem]:inline">Export</span>
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => exportAs('pdf')}>PDF</DropdownMenuItem>
          <DropdownMenuItem onSelect={() => exportAs('docx')}>Word (.docx)</DropdownMenuItem>
          <DropdownMenuItem onSelect={() => exportAs('md')}>Markdown</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuCheckboxItem
            checked={includeTranscript}
            onCheckedChange={v => setIncludeTranscript(!!v)}
            onSelect={e => e.preventDefault()}
          >
            Include transcript
          </DropdownMenuCheckboxItem>
        </DropdownMenuContent>
      </DropdownMenu>

    </ButtonGroup>
  );
}
