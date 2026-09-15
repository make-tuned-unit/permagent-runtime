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
    if (cached && cachedRev === identityRev) { setName(cached); return; }
    let cancelled = false;
    let retry: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      try {
        const id = await api.getIdentity();
        if (cancelled) return;
        cached = id.first_name;
        cachedRev = identityRev;
        setName(id.first_name);
      } catch {
        // The local daemon may still be starting after a restart.
        if (!cancelled) retry = setTimeout(refresh, 5000);
      }
    }
    void refresh();
    return () => { cancelled = true; clearTimeout(retry); };
  }, [identityRev]);
  return name;
}
