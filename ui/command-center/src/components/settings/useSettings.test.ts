// Issue #167 — persona settings load error + save persistence.
// Pins the payload builder that makes Save work even when the initial GET
// failed (the old code silently no-oped on `data === null`, so the user's
// edits were dropped while "Failed to load persona" sat next to the button).
import { describe, it, expect } from 'vitest';
import { buildIdentityPayload, type PersonaData } from './useSettings';

const loaded: PersonaData = {
  first_name: 'Aria',
  last_name: null,
  nickname: 'Ari',
  display_name: 'Ari',
  traits: ['calm'],
  tone: 'warm',
  opening_greeting: 'Hello!',
  voice_id: 'af_heart',
};

describe('buildIdentityPayload', () => {
  it('merges form edits over the loaded persona', () => {
    const payload = buildIdentityPayload(loaded, {
      first_name: 'Henry',
      opening_greeting: 'Hey boss!',
    });
    expect(payload).toEqual({
      first_name: 'Henry',
      traits: ['calm'],
      tone: 'warm',
      opening_greeting: 'Hey boss!',
      last_name: null,
      nickname: 'Ari',
      voice_id: 'af_heart',
    });
  });

  it('builds a full payload from form values alone when the load failed (#167)', () => {
    const payload = buildIdentityPayload(null, {
      first_name: 'Henry',
      opening_greeting: 'Hey boss!',
      tone: 'direct',
      traits: ['direct'],
      voice_id: null,
    });
    expect(payload).toEqual({
      first_name: 'Henry',
      traits: ['direct'],
      tone: 'direct',
      opening_greeting: 'Hey boss!',
      last_name: null,
      nickname: null,
      voice_id: null,
    });
  });

  it('explicit null clears an optional field; omission keeps the loaded value', () => {
    const cleared = buildIdentityPayload(loaded, { nickname: null });
    expect(cleared?.nickname).toBeNull();
    const kept = buildIdentityPayload(loaded, {});
    expect(kept?.nickname).toBe('Ari');
  });

  it('returns null (unsavable) only when no name exists anywhere', () => {
    expect(buildIdentityPayload(null, {})).toBeNull();
    expect(buildIdentityPayload(null, { first_name: '   ' })).toBeNull();
    expect(buildIdentityPayload(loaded, {})?.first_name).toBe('Aria');
  });

  it('trims the name', () => {
    expect(buildIdentityPayload(null, { first_name: '  Henry ' })?.first_name).toBe('Henry');
  });
});
