// The orchestrator's configured display name — resolved from the SAME source
// ChatLauncher uses (getIdentity().first_name, e.g. "Henry"). The static roster
// hardcodes a placeholder ('Aria'); resolving through this hook keeps the 3D
// nameplate, hover tooltip, and Henry HUD from leaking the default persona key.
// Identity is config, not a literal (bible §4). Module-cached so the five
// inhabitants + HUD share a single fetch.
import { useEffect, useState } from 'react';
import { api } from '../../../lib/api';

let cached: string | null = null;

export function useOrchestratorName(): string | null {
  const [name, setName] = useState<string | null>(cached);
  useEffect(() => {
    if (cached) return;
    let cancelled = false;
    api
      .getIdentity()
      .then((id) => {
        cached = id.first_name;
        if (!cancelled) setName(id.first_name);
      })
      .catch(() => {
        // Identity unavailable — callers fall back to their own default.
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return name;
}
