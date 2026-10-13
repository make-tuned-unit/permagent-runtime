import { useCallback, useState } from 'react';
import type { Appearance } from './bakedEnvironment';

type Readiness = { requested: Appearance; appearance: Appearance; applied: Appearance | null;
  skyReady: boolean; environmentReady: boolean; error: boolean; attempt: number };
/** Selection is immediate; only the current request can acknowledge resources
 * and its first rendered frame. Stale load completions cannot change the scene. */
export function useForumLightingReadiness(requested: Appearance) {
  const [state, setState] = useState<Readiness>({ requested, appearance: requested,
    applied: null, skyReady: false, environmentReady: false, error: false, attempt: 0 });
  // Reset readiness in the same render as selection, including day→night→day.
  if (state.requested !== requested) setState({ ...state, requested, skyReady: false, environmentReady: false, error: false });
  const prepared = useCallback((next: Appearance) => setState(s => s.requested !== next ? s : {
    ...s, appearance: next, skyReady: true, error: false, applied: s.environmentReady ? next : s.applied,
  }), []);
  const didApply = useCallback((next: Appearance) => setState(s => s.requested !== next ? s : {
    ...s, environmentReady: true, applied: s.skyReady ? next : s.applied,
  }), []);
  const failed = useCallback((next: Appearance) => setState(s => s.requested !== next ? s : {
    ...s, error: true, appearance: s.applied ?? s.appearance,
  }), []);
  const retry = useCallback(() => setState(s => ({ ...s, error: false, skyReady: false, environmentReady: false, attempt: s.attempt + 1 })), []);
  return { ...state, prepared, didApply, failed, retry,
    pending: !state.error && (!state.skyReady || !state.environmentReady || state.applied !== requested || state.appearance !== requested),
  };
}
