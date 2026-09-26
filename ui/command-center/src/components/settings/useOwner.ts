import { useState, useEffect, useCallback } from 'react';
import { api, type OwnerResponse } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';

/**
 * The owner identity: the human running this Permagent instance, as distinct
 * from `usePersona()` (the agent's own name/voice/tone). Mirrors that hook's
 * shape and contract exactly — same cache-then-refresh pattern, same
 * "only clear dirty on a confirmed save" rule (#167's lesson).
 */

/** Last owner record this window fetched, shared across every `useOwner`
 *  mount — mirrors `personaCache` in useSettings.ts. */
let ownerCache: OwnerResponse | null = null;

export function useOwner() {
  const [data, setData] = useState<OwnerResponse | null>(ownerCache);
  const [loading, setLoading] = useState(ownerCache === null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const res = await api.getOwner();
      ownerCache = res;
      setData(res);
      setError(null);
    } catch {
      setError('Failed to load owner info');
    }
    setLoading(false);
  }, []);

  // The owner record is not on the generic Config key/value store, but
  // `PUT /api/owner` emits the same `config_changed` frame Settings already
  // refetches on (see routes/owner.rs) — reusing `configRev` keeps this in
  // one wiring rather than a second event kind for one hook.
  const configRev = useCommandCenter(s => s.configRev);
  useEffect(() => { load(); }, [load, configRev]);

  /** Retry a failed initial load. */
  const reload = useCallback(async () => {
    setLoading(true);
    await load();
  }, [load]);

  /**
   * Save owner edits. Returns true only when the daemon persisted them —
   * callers must not clear their dirty state on false (#167).
   */
  const save = useCallback(async (update: {
    display_name?: string | null; email?: string | null; linked_person_id?: string | null;
  }): Promise<boolean> => {
    setSaving(true);
    setError(null);
    try {
      const saved = await api.putOwner(update);
      ownerCache = saved;
      setData(saved);
      setSaving(false);
      return true;
    } catch (e) {
      const message = e instanceof Error ? e.message : '';
      setError(message || 'Failed to save owner info');
      setSaving(false);
      return false;
    }
  }, []);

  return { data, loading, saving, error, save, reload };
}
