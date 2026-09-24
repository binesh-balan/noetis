'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from './ui/button';

interface EntraStatus { enabled: boolean; signedIn: boolean; account?: string | null }

// Work-account sign-in used for AI summaries when the organization uses keyless Azure access.
// Renders nothing unless the managed policy enables it.
export function OrgAccountSettings() {
  const [status, setStatus] = useState<EntraStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<EntraStatus>('api_entra_status'));
    } catch (e) {
      console.error('Failed to read organization sign-in status:', e);
    }
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  const run = async (command: 'api_entra_sign_in' | 'api_entra_sign_out') => {
    setBusy(true);
    try {
      await invoke(command);
      if (command === 'api_entra_sign_in') toast.success('Signed in to your organization account');
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
      refresh();
    }
  };

  if (!status?.enabled) return null;

  return (
    <div className="bg-background rounded-lg border border-border p-6 shadow-sm mb-4 flex items-center justify-between gap-4">
      <div>
        <h3 className="text-lg font-semibold text-foreground">Organization account</h3>
        <p className="text-sm text-muted-foreground">
          {status.signedIn
            ? `AI summaries use your work account${status.account ? ` (${status.account})` : ''}.`
            : 'Sign in with your work account to enable AI summaries.'}
        </p>
      </div>
      {status.signedIn ? (
        <Button variant="outline" disabled={busy} onClick={() => run('api_entra_sign_out')}>Sign out</Button>
      ) : (
        <Button disabled={busy} onClick={() => run('api_entra_sign_in')}>{busy ? 'Waiting for browser...' : 'Sign in'}</Button>
      )}
    </div>
  );
}
