import { useEffect, useState } from 'react';
import { detectVersionSkew, type VersionSkew } from '../lib/version';

/**
 * Detects app↔daemon version skew once, after the app has reached its ready
 * phase. Returns null until the check resolves. Non-blocking and self-healing:
 * a transient daemon outage just yields 'unknown' (no banner).
 */
export function useVersionSkew(enabled: boolean): VersionSkew | null {
  const [skew, setSkew] = useState<VersionSkew | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    detectVersionSkew().then(result => {
      if (!cancelled) setSkew(result);
    });
    return () => { cancelled = true; };
  }, [enabled]);

  return skew;
}
