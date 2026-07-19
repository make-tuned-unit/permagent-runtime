import { useState, useEffect, useCallback } from 'react';
import { api } from '../../lib/api';
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

export function usePersona() {
  const [data, setData] = useState<PersonaData | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const res = await api.getIdentity();
      setData(res as PersonaData);
      setError(null);
    } catch {
      setError('Failed to load persona');
    }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load]);

  const save = useCallback(async (update: Partial<PersonaData>) => {
    if (!data) return;
    setSaving(true);
    setError(null);
    try {
      const payload = {
        first_name: update.first_name ?? data.first_name,
        traits: update.traits ?? data.traits,
        tone: update.tone ?? data.tone,
        opening_greeting: update.opening_greeting ?? data.opening_greeting,
        last_name: update.last_name !== undefined ? update.last_name : data.last_name,
        nickname: update.nickname !== undefined ? update.nickname : data.nickname,
        voice_id: update.voice_id !== undefined ? update.voice_id : data.voice_id,
      };
      await api.putIdentity(payload);
      // The user configured their agent's identity — genuine engagement with
      // the persona feature, so the coach stops offering to teach it.
      emitActivity('persona_configured', 'settings', { persona_name: payload.first_name });
      await load();
    } catch {
      setError('Failed to save persona');
    }
    setSaving(false);
  }, [data, load]);

  return { data, loading, saving, error, save };
}
