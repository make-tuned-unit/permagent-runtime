import { describe, expect, it } from 'vitest';
import {
  availabilityLabel,
  dispatchAvailabilityLabel,
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
  firstSentence,
  workerSummary,
  liveStatusChip,
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

describe('dispatchAvailabilityLabel', () => {
  it('does not call a pending engine dispatchable when its prerequisite probe passes', () => {
    expect(dispatchAvailabilityLabel({ engine: 'pending', availability: { status: 'available' } })).toEqual({
      text: 'Not dispatchable — no runnable engine', tone: 'unknown',
    });
  });

  it('distinguishes an internal registration from model or task verification', () => {
    const status = dispatchAvailabilityLabel({ engine: 'internal_subagent', availability: { status: 'available' } });
    expect(status.text).toBe('Registered; model and task unverified');
    expect(status.tone).toBe('unknown');
  });

  it('preserves failed prerequisite detail even for a pending engine', () => {
    const status = dispatchAvailabilityLabel({ engine: 'pending', availability: { status: 'unavailable', reason: 'binary missing' } });
    expect(status.text).toContain('Not dispatchable');
    expect(status.text).toContain('binary missing');
    expect(status.tone).toBe('error');
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

describe('workerSummary', () => {
  // Ruling 2026-09-25's 1440px review of the real daemon: worker rows were printing the
  // full engineering paragraph (Scheduler's guardrail internals, Git
  // Steward's config keys, ...) because there was no separate short field.
  it('prefers the daemon-curated summary over the long description', () => {
    expect(workerSummary({
      summary: 'Runs your recurring automations and reminders on schedule.',
      what_it_does: 'Runs saved recipes and reminders in the background on cron, one-time, or interval schedules — retrying transient failures, recovering runs missed during downtime, and escalating repeated failures to the Decision Inbox. Guardrails are enforced, not advisory: schedules cannot fire more often than every 15 minutes.',
    })).toBe('Runs your recurring automations and reminders on schedule.');
  });

  it('falls back to the first sentence for a daemon that predates summary', () => {
    expect(workerSummary({
      what_it_does: 'The agent that owns repo hygiene and CI. It runs on its own schedule.',
    })).toBe('The agent that owns repo hygiene and CI.');
  });

  it('treats an empty-string summary the same as a missing one', () => {
    expect(workerSummary({ summary: '  ', what_it_does: 'Does the thing. And more.' }))
      .toBe('Does the thing.');
  });
});

describe('firstSentence', () => {
  it('cuts at the first sentence boundary', () => {
    expect(firstSentence('First bit. Second bit. Third bit.')).toBe('First bit.');
  });

  it('returns the whole string, with a period, when there is no sentence boundary', () => {
    expect(firstSentence('No period anywhere here')).toBe('No period anywhere here.');
  });

  it('caps a single very long clause rather than leaking the whole paragraph', () => {
    const long = 'x'.repeat(300);
    const result = firstSentence(long);
    expect(result.length).toBeLessThanOrEqual(161);
    expect(result.endsWith('...')).toBe(true);
  });
});

describe('liveStatusChip', () => {
  // Raw daemon values remain in the tooltip, while a count or enabled flag
  // must never be promoted to evidence of an executing task.
  it('narrows a config-key-leaking off value to plain "Off"', () => {
    const chip = liveStatusChip({ status: 'ok', value: 'off (strix_enabled=false)' });
    expect(chip.label).toBe('Off');
    expect(chip.detail).toContain('strix_enabled=false');
  });

  it('narrows a bare "on" to a neutral enabled fact, not Running', () => {
    const chip = liveStatusChip({ status: 'ok', value: 'on' });
    expect(chip.label).toBe('Enabled');
    expect(chip.tone).toBe('unknown');
  });

  it('reports configured job counts without implying paused jobs will run', () => {
    expect(liveStatusChip({ status: 'ok', value: '3 jobs' }).label).toBe('3 jobs configured');
    expect(liveStatusChip({ status: 'ok', value: '1 job' }).label).toBe('1 job configured');
    expect(liveStatusChip({ status: 'ok', value: '5 job(s) scheduled' }).label).toBe('5 jobs configured');
    expect(liveStatusChip({ status: 'ok', value: '5 job(s) scheduled' }).tone).toBe('unknown');
  });

  it('maps "idle" straight through', () => {
    expect(liveStatusChip({ status: 'ok', value: 'idle' }).label).toBe('Idle');
  });

  it('preserves an explicit running signal but treats counts and enabled flags as non-running', () => {
    expect(liveStatusChip({ status: 'ok', value: 'running a scan' }).label).toBe('Running');
    expect(liveStatusChip({ status: 'ok', value: '0 described, 0 pending' }).label).toBe('Status reported');
    expect(liveStatusChip({ status: 'ok', value: 'git-health sweep off (steward_scan_enabled=false)' }, { config_key: 'steward_scan_enabled', enabled: false }).label).toBe('Off');
    expect(liveStatusChip({ status: 'ok', value: 'running an already-started scan' }, { config_key: 'steward_scan_enabled', enabled: false }).label).toBe('Running');
  });

  it('maps not_queryable to No live signal and unavailable to Unknown, distinctly', () => {
    expect(liveStatusChip({ status: 'not_queryable' }).label).toBe('No live signal');
    const unavailable = liveStatusChip({ status: 'unavailable', reason: 'pool locked' });
    expect(unavailable.label).toBe('Unknown');
    expect(unavailable.tone).toBe('error');
    expect(unavailable.detail).toContain('pool locked');
  });

  it('never renders the raw daemon string as the label itself', () => {
    for (const s of [
      { status: 'ok', value: 'off (strix_enabled=false)' } as const,
      { status: 'ok', value: 'on' } as const,
      { status: 'not_queryable' } as const,
      { status: 'unavailable', reason: 'x' } as const,
    ]) {
      const chip = liveStatusChip(s);
      expect(['Enabled', 'No live signal', 'Off', 'Unknown']).toContain(chip.label);
    }
  });
});
