import { useState, useEffect, useCallback } from 'react';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { emitActivity } from '../../lib/emitActivity';

export interface PersonaData {
  first_name: string;
  last_name: string | null;
  nickname: string | null;
  display_name: string;
  traits: string[];
  tone: string;
  opening_greeting: string;
  voice_id: string | null;
}

export type IdentityPayload = Parameters<typeof api.putIdentity>[0];

/**
 * Build the PUT /api/agent/identity payload from the loaded persona (if any)
 * merged with the form's edits.
 *
 * Issue #167: when the initial load failed, `save()` used to silently no-op
 * (`if (!data) return`) — the Save button looked live but did nothing, and the
 * "Failed to load persona" error sat next to it while edits were dropped.
 * The form's own values are a sufficient payload, so a failed load must not
 * disable saving. Returns null only when there is no usable first_name at all.
 */
export function buildIdentityPayload(
  data: PersonaData | null,
  update: Partial<PersonaData>,
): IdentityPayload | null {
  const first_name = (update.first_name ?? data?.first_name ?? '').trim();
  if (!first_name) return null;
  return {
    first_name,
    traits: update.traits ?? data?.traits ?? [],
    tone: update.tone ?? data?.tone ?? '',
    opening_greeting: update.opening_greeting ?? data?.opening_greeting ?? '',
    last_name: update.last_name !== undefined ? update.last_name : (data?.last_name ?? null),
    nickname: update.nickname !== undefined ? update.nickname : (data?.nickname ?? null),
    voice_id: update.voice_id !== undefined ? update.voice_id : (data?.voice_id ?? null),
  };
}

/** Last identity this window fetched, shared across every usePersona mount.
 *  Each mount used to start from null and refetch — so copy rendered before
 *  that component's own fetch resolved fell back to a hardcoded name ("Aria
 *  will file this" while the agent is Henry). New mounts now start from the
 *  cache and refresh in the background. */
let personaCache: PersonaData | null = null;

export function usePersona() {
  const [data, setData] = useState<PersonaData | null>(personaCache);
  const [loading, setLoading] = useState(personaCache === null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const res = await api.getIdentity();
      personaCache = res as PersonaData;
      setData(res as PersonaData);
      setError(null);
    } catch {
      setError('Failed to load persona');
    }
    setLoading(false);
  }, []);

  // #629 multi-client liveness: identityRev bumps when `identity_changed`
  // arrives on /events — the persona form re-reads a remote edit.
  const identityRev = useCommandCenter(s => s.identityRev);
  useEffect(() => { load(); }, [load, identityRev]);

  /** Retry a failed initial load (shows a spinner again). */
  const reload = useCallback(async () => {
    setLoading(true);
    await load();
  }, [load]);

  /**
   * Save persona edits. Returns true only when the daemon persisted them —
   * callers must not clear their dirty state on false.
   *
   * The PUT response IS the fresh persona, so state updates from it directly
   * instead of a dependent re-GET (#167: a save that succeeded could still
   * surface "Failed to load persona" and appear to revert).
   */
  const save = useCallback(async (update: Partial<PersonaData>): Promise<boolean> => {
    setSaving(true);
    setError(null);
    const payload = buildIdentityPayload(data, update);
    if (!payload) {
      setError('Give your agent a name before saving');
      setSaving(false);
      return false;
    }
    try {
      const saved = await api.putIdentity(payload);
      personaCache = saved as PersonaData;
      setData(saved as PersonaData);
      // The user configured their agent's identity — genuine engagement with
      // the persona feature, so the coach stops offering to teach it.
      emitActivity('persona_configured', 'settings', { persona_name: payload.first_name });
      setSaving(false);
      return true;
    } catch {
      setError('Failed to save persona');
      setSaving(false);
      return false;
    }
  }, [data]);

  return { data, loading, saving, error, save, reload };
}
