import { describe, expect, it } from 'vitest';
import {
  availabilityLabel,
  emptyWorkNote,
  EMPTY_ACTIVITY_NOTE,
  grantsSummary,
  liveStateLabel,
  presenceLabel,
  requiredSecretsLabel,
  defaultEnabledLabel,
  NOT_DECLARED_SECRETS,
  NO_DEFAULT_DECLARED,
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

describe('emptyWorkNote', () => {
  it('uses attribution wording, never "did nothing" / "no work yet"', () => {
    const note = emptyWorkNote({
      attribution: 'actor_exact_match',
      status: 'ok',
      items: [],
      truncated: false,
    });
    expect(note).toBe(EMPTY_ACTIVITY_NOTE);
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

  it('renders null default as no default declared, never off', () => {
    expect(defaultEnabledLabel(null)).toBe(NO_DEFAULT_DECLARED);
    expect(defaultEnabledLabel(null).toLowerCase()).not.toBe('off');
  });
});
