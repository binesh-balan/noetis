import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface ManagedPolicy {
  summaryManaged: boolean;
  templatesManaged?: boolean;
  transcriptionManaged?: boolean;
  downloadsAllowed?: boolean;
  analyticsDisabled?: boolean;
  error?: string | null;
}

// Org policy (policy.json) as reported by the Rust core. Null until loaded.
export function useManagedPolicy(): ManagedPolicy | null {
  const [policy, setPolicy] = useState<ManagedPolicy | null>(null);
  useEffect(() => {
    invoke<ManagedPolicy>('api_get_managed_policy').then(setPolicy).catch(console.error);
  }, []);
  return policy;
}
