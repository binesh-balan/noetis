'use client';

import React, { useState, useEffect, Suspense } from 'react';
import { useManagedPolicy } from '@/hooks/useManagedPolicy';
import { Settings2, Mic, Database as DatabaseIcon, SparkleIcon, FlaskConical, LayoutTemplate, Palette } from 'lucide-react';
import { useRouter, useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { TranscriptSettings } from '@/components/TranscriptSettings';
import { RecordingSettings } from '@/components/RecordingSettings';
import { PreferenceSettings } from '@/components/PreferenceSettings';
import { OrgAccountSettings } from '@/components/OrgAccountSettings';
import { SummaryModelSettings } from '@/components/SummaryModelSettings';
import { TemplateSettings } from '@/components/TemplateSettings';
import { BetaSettings } from '@/components/BetaSettings';
import { AppearanceSettings } from '@/components/AppearanceSettings';
import { useConfig } from '@/contexts/ConfigContext';

// Tabs configuration (constant)
const TABS = [
  { value: 'general', label: 'General', icon: Settings2 },
  { value: 'recording', label: 'Recordings', icon: Mic },
  { value: 'Transcriptionmodels', label: 'Transcription', icon: DatabaseIcon },
  { value: 'summaryModels', label: 'Summary', icon: SparkleIcon },
  { value: 'templates', label: 'Templates', icon: LayoutTemplate },
  { value: 'appearance', label: 'Appearance', icon: Palette },
  { value: 'beta', label: 'Beta', icon: FlaskConical }
] as const;

// useSearchParams needs a Suspense boundary under static export.
export default function SettingsPage() {
  return (
    <Suspense fallback={null}>
      <SettingsContent />
    </Suspense>
  );
}

function SettingsContent() {
  const tabParam = useSearchParams().get('tab');
  const router = useRouter();
  const { transcriptModelConfig, setTranscriptModelConfig } = useConfig();
  // Org-managed transcription: hide the tab entirely.
  const transcriptionManaged = useManagedPolicy()?.transcriptionManaged ?? false;
  const tabs = TABS.filter(t => !(transcriptionManaged && t.value === 'Transcriptionmodels'));

  // Deep link: /settings?tab=<value>. Re-sync when the param changes while mounted.
  const [selectedTab, setActiveTab] = useState<string>(tabParam ?? 'general');
  useEffect(() => { setActiveTab(tabParam ?? 'general'); }, [tabParam]);
  // Unknown or policy-hidden tab (policy may load after mount) falls back to General.
  const activeTab = tabs.some(t => t.value === selectedTab) ? selectedTab : 'general';

  // Load saved transcript configuration on mount
  useEffect(() => {
    const loadTranscriptConfig = async () => {
      try {
        const config = await invoke('api_get_transcript_config') as any;
        if (config) {
          console.log('Loaded saved transcript config:', config);
          setTranscriptModelConfig({
            provider: config.provider || 'localWhisper',
            model: config.model || 'large-v3',
            apiKey: config.apiKey || null
          });
        }
      } catch (error) {
        console.error('Failed to load transcript config:', error);
      }
    };
    loadTranscriptConfig();
  }, [setTranscriptModelConfig]);

  return (
    <div className="flex h-full">
      <nav aria-label="Settings sections" className="w-48 shrink-0 space-y-0.5 border-r border-border p-3">
        <h1 className="px-2 pb-3 text-sm font-semibold">Settings</h1>
        {tabs.map(({ value, label, icon: Icon }) => (
          <button
            key={value}
            onClick={() => {
              setActiveTab(value);
              router.replace('/settings?tab=' + value);
            }}
            aria-current={activeTab === value ? 'page' : undefined}
            className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${
              activeTab === value ? 'bg-muted text-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'
            }`}
          >
            <Icon className="h-4 w-4" /> {label}
          </button>
        ))}
      </nav>
      <div className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl space-y-6 p-8">
          {activeTab === 'general' && (<><OrgAccountSettings /><PreferenceSettings /></>)}
          {activeTab === 'recording' && <RecordingSettings />}
          {activeTab === 'Transcriptionmodels' && (
            <TranscriptSettings transcriptModelConfig={transcriptModelConfig} setTranscriptModelConfig={setTranscriptModelConfig} />
          )}
          {activeTab === 'summaryModels' && <SummaryModelSettings />}
          {activeTab === 'templates' && <TemplateSettings />}
          {activeTab === 'beta' && <BetaSettings />}
          {activeTab === 'appearance' && <AppearanceSettings />}
        </div>
      </div>
    </div>
  );
}
