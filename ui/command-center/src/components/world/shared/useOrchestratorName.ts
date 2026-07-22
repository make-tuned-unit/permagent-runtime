// The orchestrator's configured display name — resolved from the SAME source
// ChatLauncher uses (getIdentity().first_name, e.g. "Henry"). The roster carries
// a static default name; resolving through this hook keeps the 3D nameplate,
// hover tooltip, and Henry HUD showing the live configured persona name rather
// than the default. Identity is config, not a literal. Module-cached so the
// inhabitants + HUD share a single fetch — and the cache is keyed to the
// store's identityRev (#629), so an `identity_changed` event from another
// device invalidates it and the nameplate re-reads the new name live.
import { useEffect, useState } from 'react';
import { api } from '../../../lib/api';
import { useCommandCenter } from '../../../lib/store';

let cached: string | null = null;
let cachedRev = 0;

export function useOrchestratorName(): string | null {
  const identityRev = useCommandCenter(s => s.identityRev);
  const [name, setName] = useState<string | null>(cached);
  useEffect(() => {
    if (cached && cachedRev === identityRev) return;
    let cancelled = false;
    api
      .getIdentity()
      .then((id) => {
        cached = id.first_name;
        cachedRev = identityRev;
        if (!cancelled) setName(id.first_name);
      })
      .catch(() => {
        // Identity unavailable — callers fall back to their own default.
      });
    return () => {
      cancelled = true;
    };
  }, [identityRev]);
  return name;
}
