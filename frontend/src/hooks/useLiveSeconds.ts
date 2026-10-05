'use client';

import { useState, useEffect } from 'react';

/**
 * `activeDuration` (seconds) only changes when RecordingStateContext syncs with the
 * backend (500ms polling after a start event; a single sync after a reload). Tick
 * locally from the last known value while recording and not paused; freeze on pause.
 */
export function useLiveSeconds(activeDuration: number | null, ticking: boolean): number {
  const [base, setBase] = useState({ value: activeDuration ?? 0, at: Date.now() });
  const [, setTick] = useState(0);
  useEffect(() => setBase({ value: activeDuration ?? 0, at: Date.now() }), [activeDuration, ticking]);
  useEffect(() => {
    if (!ticking) return;
    const id = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, [ticking]);
  return ticking ? base.value + (Date.now() - base.at) / 1000 : base.value;
}
