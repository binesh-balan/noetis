import React, { useState, useEffect } from "react";
import { getVersion } from '@tauri-apps/api/app';
import { useManagedPolicy } from '@/hooks/useManagedPolicy';
import Image from 'next/image';
import AnalyticsConsentSwitch from "./AnalyticsConsentSwitch";
import { UpdateDialog } from "./UpdateDialog";
import { updateService, UpdateInfo } from '@/services/updateService';
import { Button } from './ui/button';
import {
    Loader2,
    RefreshCw,
    Radar,
    AudioLines,
    Users,
    ListChecks,
    Cpu,
    ShieldCheck,
    type LucideIcon,
} from 'lucide-react';
import { toast } from 'sonner';

interface Feature {
    icon: LucideIcon;
    title: string;
    body: string;
}

const FEATURES: Feature[] = [
    {
        icon: Radar,
        title: 'Catches your calls',
        body: 'On Windows, notices Zoom, Teams, Meet, Slack and more using your mic, asks to record, and stops when the call ends.',
    },
    {
        icon: AudioLines,
        title: 'Live transcript',
        body: 'Microphone and system audio transcribed as you talk, with Parakeet or Whisper running on your machine.',
    },
    {
        icon: Users,
        title: 'Knows who spoke',
        body: 'Separates speakers after the meeting and remembers voices, so names carry over to the next call.',
    },
    {
        icon: ListChecks,
        title: 'Summaries & action items',
        body: 'Key decisions, action items and highlights, from templates you can adapt to any kind of meeting.',
    },
    {
        icon: Cpu,
        title: 'Your choice of model',
        body: 'A built-in local model or Ollama, or bring Claude, Groq or OpenRouter when you want them.',
    },
    {
        icon: ShieldCheck,
        title: 'Private by default',
        body: 'Audio, transcripts and voiceprints stay on this computer. Nothing is uploaded unless you choose a cloud model.',
    },
];

export function About() {
    const analyticsLocked = useManagedPolicy()?.analyticsDisabled ?? false;
    const [currentVersion, setCurrentVersion] = useState<string>('0.4.1');
    const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
    const [isChecking, setIsChecking] = useState(false);
    const [showUpdateDialog, setShowUpdateDialog] = useState(false);

    useEffect(() => {
        getVersion().then(setCurrentVersion).catch(console.error);
    }, []);

    const handleCheckForUpdates = async () => {
        setIsChecking(true);
        try {
            const info = await updateService.checkForUpdates(true);
            setUpdateInfo(info);
            if (info.available) {
                setShowUpdateDialog(true);
            } else {
                toast.success('You are running the latest version');
            }
        } catch (error: any) {
            console.error('Failed to check for updates:', error);
            toast.error('Failed to check for updates: ' + (error.message || 'Unknown error'));
        } finally {
            setIsChecking(false);
        }
    };

    return (
        <div className="max-h-[80vh] space-y-6 overflow-y-auto p-1">
            {/* Identity */}
            <header className="flex items-center gap-4">
                <Image
                    src="icon_128x128.png"
                    alt=""
                    width={56}
                    height={56}
                    className="h-14 w-14 shrink-0 rounded-xl"
                />
                <div className="min-w-0 flex-1">
                    <div className="flex items-baseline gap-2">
                        <h1 className="text-2xl font-semibold tracking-tight text-foreground">Noetis</h1>
                        <span className="rounded-full border border-border px-2 py-0.5 font-mono text-[11px] text-muted-foreground">
                            v{currentVersion}
                        </span>
                    </div>
                    <p className="mt-1 text-sm text-muted-foreground">
                        Your meeting assistant that never leaves your machine.
                    </p>
                </div>
            </header>

            <div className="flex flex-wrap items-center gap-3">
                <Button onClick={handleCheckForUpdates} disabled={isChecking} variant="outline" size="sm">
                    {isChecking ? (
                        <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                    ) : (
                        <RefreshCw className="mr-2 h-3.5 w-3.5" />
                    )}
                    {isChecking ? 'Checking…' : 'Check for updates'}
                </Button>
                {updateInfo?.available && (
                    <button
                        type="button"
                        onClick={() => setShowUpdateDialog(true)}
                        className="text-sm font-medium text-primary underline-offset-4 hover:underline"
                    >
                        v{updateInfo.version} is available
                    </button>
                )}
            </div>

            {/* What it does */}
            <section aria-labelledby="about-features">
                <h2
                    id="about-features"
                    className="mb-3 text-xs font-medium uppercase tracking-wider text-muted-foreground"
                >
                    What Noetis does
                </h2>
                <ul className="grid gap-x-6 gap-y-5 sm:grid-cols-2">
                    {FEATURES.map(({ icon: Icon, title, body }) => (
                        <li key={title} className="flex gap-3">
                            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
                                <Icon className="h-4 w-4" aria-hidden="true" />
                            </span>
                            <div>
                                <h3 className="text-sm font-medium text-foreground">{title}</h3>
                                <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{body}</p>
                            </div>
                        </li>
                    ))}
                </ul>
            </section>

            {!analyticsLocked && (
                <div className="border-t border-border pt-4">
                    <AnalyticsConsentSwitch />
                </div>
            )}

            <UpdateDialog
                open={showUpdateDialog}
                onOpenChange={setShowUpdateDialog}
                updateInfo={updateInfo}
            />
        </div>
    );
}
