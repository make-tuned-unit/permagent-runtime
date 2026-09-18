import { describe, expect, it } from 'vitest';
import {
  availabilityLabel,
  grantsNotEnforcedNote,
  EMPTY_ACTIVITY_NOTE,
  grantsSummary,
  liveStateLabel,
  presenceLabel,
  requiredSecretsLabel,
  requiredSecretHint,
  requiredSecretHints,
  defaultEnabledLabel,
  NOT_DECLARED_SECRETS,
  NO_DEFAULT_DECLARED,
  NO_AGENT_SECRETS_NOTE,
  STORED_SECRETS_NOTE,
  gateRowHint,
  readAgentGate,
} from './agentsPanel';

describe('liveStateLabel', () => {
  it('maps ok to the value with ok tone', () => {
    const l = liveStateLabel({ status: 'ok', value: '3 jobs' });
    expect(l).toEqual({ text: '3 jobs', tone: 'ok' });
  });

  it('maps not_queryable as a property, not a failure', () => {
    const l = liveStateLabel({ status: 'not_queryable' });
    expect(l.text).toBe('no live state to query');
    expect(l.tone).toBe('unknown');
    expect(l.text.toLowerCase()).not.toMatch(/\bidle\b|\bok\b/);
  });

  it('maps unavailable with reason and never as idle/ok', () => {
    const l = liveStateLabel({ status: 'unavailable', reason: 'pool locked' });
    expect(l.text).toContain('pool locked');
    expect(l.text).toMatch(/could not be read/i);
    expect(l.tone).toBe('error');
    expect(l.text.toLowerCase()).not.toMatch(/\bidle\b/);
    expect(l.text.toLowerCase()).not.toBe('ok');
    expect(l.text.toLowerCase()).not.toContain('idle');
  });
});

describe('availabilityLabel', () => {
  it('maps available', () => {
    expect(availabilityLabel({ status: 'available' })).toEqual({
      text: 'available',
      tone: 'ok',
    });
  });

  it('maps unavailable with reason', () => {
    const l = availabilityLabel({ status: 'unavailable', reason: 'bin missing' });
    expect(l.text).toContain('bin missing');
    expect(l.text).toMatch(/unavailable/i);
    expect(l.tone).toBe('error');
    expect(l.text.toLowerCase()).not.toMatch(/\bidle\b|\bok\b/);
  });

  it('maps probe_failed as could not check, not unavailable', () => {
    const l = availabilityLabel({ status: 'probe_failed', reason: 'timeout' });
    expect(l.text).toContain('timeout');
    expect(l.text).toMatch(/could not check/i);
    expect(l.text.toLowerCase()).not.toContain('unavailable');
    expect(l.tone).toBe('error');
  });
});

describe('presenceLabel', () => {
  it('handles present / absent strings', () => {
    expect(presenceLabel('present')).toEqual({ text: 'present', tone: 'ok' });
    expect(presenceLabel('absent')).toEqual({ text: 'absent', tone: 'unknown' });
  });

  it('carries the unreadable reason', () => {
    const l = presenceLabel({ unreadable: 'store locked' });
    expect(l.text).toContain('store locked');
    expect(l.text).toMatch(/unreadable/i);
    expect(l.tone).toBe('error');
  });
});

describe('grantsSummary', () => {
  it('summarizes inherit / nothing / narrowed', () => {
    expect(
      grantsSummary({
        grants: { mode: 'inherit_global' },
        grants_enforced: true,
      }),
    ).toMatch(/inherits/i);
    expect(
      grantsSummary({
        grants: { mode: 'explicit', extensions: [], truncated: false },
        grants_enforced: true,
      }),
    ).toMatch(/nothing/i);
    expect(
      grantsSummary({
        grants: { mode: 'explicit', extensions: ['grow', 'analyze'], truncated: false },
        grants_enforced: true,
      }),
    ).toContain('grow');
  });

  it('states when grants are recorded but not enforced', () => {
    const text = grantsSummary({
      grants: { mode: 'inherit_global' },
      grants_enforced: false,
    });
    expect(text.toLowerCase()).toMatch(/not enforced/);
    expect(text.toLowerCase()).toMatch(/recorded/);
  });
});

describe('grantsNotEnforcedNote', () => {
  it('blames the CLI process only for CLI engines', () => {
    for (const engine of ['external_cli', 'supervised_cli']) {
      const note = grantsNotEnforcedNote(engine);
      expect(note.toLowerCase()).toContain('cli process');
      expect(note.toLowerCase()).toContain('not enforced');
    }
  });

  it('does not claim a pending persona runs a CLI process', () => {
    const note = grantsNotEnforcedNote('pending');
    expect(note.toLowerCase()).not.toContain('cli process');
    expect(note.toLowerCase()).toContain('no runnable engine');
  });

  it('still says grants are unenforced for an engine it does not know', () => {
    expect(grantsNotEnforcedNote('some_future_engine').toLowerCase()).toContain('not enforced');
  });
});

describe('EMPTY_ACTIVITY_NOTE', () => {
  it('uses attribution wording, never "did nothing" / "no work yet"', () => {
    const note = EMPTY_ACTIVITY_NOTE;
    expect(note.toLowerCase()).toContain('attributed');
    expect(note.toLowerCase()).toContain('not proof the agent did nothing');
    expect(note.toLowerCase()).not.toContain('no work yet');
    // Must not assert idle as a positive claim.
    expect(note.toLowerCase()).not.toMatch(/\bthis agent did nothing\b/);
  });
});

describe('requiredSecretsLabel / defaultEnabledLabel', () => {
  it('never claims not_declared means needs no secrets', () => {
    expect(requiredSecretsLabel({ status: 'not_declared' })).toBe(NOT_DECLARED_SECRETS);
    expect(requiredSecretsLabel({ status: 'not_declared' }).toLowerCase()).not.toContain(
      'needs no secrets',
    );
  });

  it('renders impact and unlocks as one hint per secret, and nothing when neither ships', () => {
    expect(requiredSecretHint({ name: 'K', present: false })).toBeNull();
    expect(requiredSecretHint({ name: 'K', present: false, impact: 'unavailable' })).toBe(
      'K: unavailable without it',
    );
    expect(requiredSecretHint({ name: 'K', present: true, unlocks: 'Sends mail.' })).toBe(
      'K: Sends mail.',
    );
    expect(
      requiredSecretHint({ name: 'K', present: false, impact: 'degraded', unlocks: 'Sends mail.' }),
    ).toBe('K: Sends mail. — degraded without it');
  });

  it('collects hints only for secrets that carry them', () => {
    expect(requiredSecretHints({ status: 'not_declared' })).toEqual([]);
    expect(
      requiredSecretHints({
        status: 'declared',
        truncated: false,
        items: [
          { name: 'A', present: false },
          { name: 'B', present: false, impact: 'unavailable', unlocks: 'Everything.' },
        ],
      }),
    ).toEqual(['B: Everything. — unavailable without it']);
  });

  it('renders null default as no default declared, never off', () => {
    expect(defaultEnabledLabel(null)).toBe(NO_DEFAULT_DECLARED);
    expect(defaultEnabledLabel(null).toLowerCase()).not.toBe('off');
  });
});

describe('readAgentGate', () => {
  // A daemon older than this app serialises no `gate` at all. Reading that
  // absence as "off" would render a live toggle whose write lands in a key that
  // daemon never reads — a control that appears to work and does nothing.
  it('reads a missing or malformed switch as unknown, never as off', () => {
    expect(readAgentGate({})).toBeNull();
    expect(readAgentGate({ gate: null })).toBeNull();
    expect(readAgentGate({ gate: { config_key: 'x' } })).toBeNull();
    expect(readAgentGate({ gate: { config_key: 'x', enabled: 'true' } })).toBeNull();
    expect(readAgentGate({ gate: { config_key: '', enabled: false } })).toBeNull();
    expect(readAgentGate(null)).toBeNull();
    expect(readAgentGate('strix')).toBeNull();
  });

  it('parses a well-formed switch', () => {
    expect(readAgentGate({ gate: { config_key: 'strix_enabled', enabled: false } })).toEqual({
      config_key: 'strix_enabled',
      enabled: false,
    });
  });
});

describe('gateRowHint', () => {
  it('names the key and says this is the only place it is written', () => {
    const hint = gateRowHint({ config_key: 'strix_enabled', enabled: false });
    expect(hint).toContain('strix_enabled');
    expect(hint).toContain('only place');
    expect(hint).toContain('no restart');
  });

  // It used to point at "Settings → Features", the board that wrote these same
  // six keys a second time. That board is gone, and a hint naming a pane that
  // does not exist is worse than one that names none.
  it('does not send the reader to the retired Features board', () => {
    expect(gateRowHint({ config_key: 'council_enabled', enabled: true }))
      .not.toContain('Features');
  });
});

describe('per-agent secret copy', () => {
  // The product owner asked, of the Guard's page, "what am I supposed to put
  // there?". "No per-agent secrets listed." does not answer that; it implies a
  // list that could fill up. Nothing in the runtime reads these at all.
  it('says nothing reads them, not merely that none are set', () => {
    expect(NO_AGENT_SECRETS_NOTE).toContain('nothing in the runtime reads');
    expect(NO_AGENT_SECRETS_NOTE).toContain('nothing to enter');
    expect(STORED_SECRETS_NOTE).toContain('values never are');
  });
});
