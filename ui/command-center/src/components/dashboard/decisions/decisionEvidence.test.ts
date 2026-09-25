import { describe, expect, it } from 'vitest';

import {
  OPEN_EVIDENCE_LABEL, ageLabel, decisionTitle, evidenceTargetFor, humanDuration, kindLabel, plainCause, technicalDetails,
} from './decisionEvidence';
import { MOCK_PENDING_DECISIONS } from './mockDecisions';
import type { Decision } from './types';

function fixture(id: string): Decision {
  const found = MOCK_PENDING_DECISIONS.find(d => d.id === id);
  if (!found) throw new Error(`fixture ${id} not found`);
  return found;
}

describe('evidenceTargetFor', () => {
  it('resolves goal-bound decisions to dispatch evidence with a fixed label (no goal title, U2)', () => {
    expect(evidenceTargetFor(fixture('dec-001'))).toEqual({
      kind: 'dispatch',
      projectId: 'proj-permagent',
      goalId: 'goal-queue-saving',
      label: OPEN_EVIDENCE_LABEL,
    });
    expect(OPEN_EVIDENCE_LABEL).toBe('Open full evidence…');
  });

  it('uses council_action payload session id instead of any "latest" fallback', () => {
    const council: Decision = { ...fixture('dec-001'), kind: 'council_action', payload: { session_id: 'council-session-123' } };
    expect(evidenceTargetFor(council)).toEqual({ kind: 'council_report', councilSessionId: 'council-session-123', label: 'Open council report' });
  });

  it('returns none for council_action when payload session id is absent', () => {
    const council: Decision = { ...fixture('dec-001'), kind: 'council_action', payload: {} };
    expect(evidenceTargetFor(council).kind).toBe('none');
  });

  it('resolves S2N unblock to the pause episode target', () => {
    const d: Decision = { ...fixture('dec-004'), goal_id: null, project_id: null, payload: { target: { type: 'scheduled_job', job_id: 'job-7' }, pause_episode_id: 'ep-9' } };
    expect(evidenceTargetFor(d)).toMatchObject({ kind: 'pause_episode', jobId: 'job-7', episodeId: 'ep-9' });
  });

  it('returns budget evidence target for reservation-gate choices', () => {
    const d: Decision = { ...fixture('dec-003'), payload: { options: [], reservation_gate: { scope: 'council', settled_usd: 1, held_usd: 2, requested_usd: 3, ceiling_usd: 10 } } };
    expect(evidenceTargetFor(d)).toMatchObject({ kind: 'budget', detail: 'council budget gate — settled $1.00, held $2.00, requested $3.00, ceiling $10.00' });
  });
});

describe('plainCause — the fallback classifier never renders raw detail (U1)', () => {
  it('turns the screenshot case into two plain sentences with no ids, seconds or salvage jargon', () => {
    const d = fixture('dec-004a-wallclock');
    const cause = plainCause(d, 'Henry');
    expect(cause).toBe('Ran out of time: each attempt gets 4 min, and this one used all of it. It made no changes worth keeping.');
    expect(cause).not.toMatch(/01a0d5ea|240s|wallclock|salvage|baseline|Last error/);
  });

  it('never contains any line of detail, for every fixture and every kind', () => {
    const kinds = ['approve_review', 'unblock', 'choice', 'risk_gate', 'automation_proposal', 'enrichment_proposal',
      'project_intel_proposal', 'file_to_project', 'council_action', 'tool_approval', 'session_gate', 'malformed',
      'model_upgrade', 'config_change_proposal', 'person_merge_proposal', 'person_delete_proposal', 'capability_gap',
      'regression_proposal', 'future_kind'];
    const marker = 'RAW-DETAIL-7f3a Goal 1234 parked: token_budget {"x":1}';
    for (const kind of kinds) {
      for (const base of MOCK_PENDING_DECISIONS) {
        const cause = plainCause({ ...base, kind, detail: marker }, 'Henry');
        expect(cause).not.toContain('RAW-DETAIL');
        expect(cause.length).toBeGreaterThan(0);
        expect(cause.split(/(?<=[.!?])\s+/).length).toBeLessThanOrEqual(2);
      }
    }
  });

  it('prefers the daemon summary when present', () => {
    expect(plainCause({ ...fixture('dec-004a-wallclock'), summary: 'Ran out of time on attempt 3.' })).toBe('Ran out of time on attempt 3.');
    expect(plainCause({ ...fixture('dec-004a-wallclock'), summary: '   ' })).toMatch(/^Ran out of time: each attempt/);
  });

  it('renders the typed unblock reasons from payload alone', () => {
    const unblock = (payload: Record<string, unknown>) => plainCause({ ...fixture('dec-004'), detail: '', payload }, 'Henry');
    expect(unblock({ reason: 'token_budget', spent: 1_200_000, cap: 1_000_000 })).toBe('Hit its spending limit: used 1.2M of 1M tokens. Henry needs your direction.');
    expect(unblock({ reason: 'attempt_cap', spent: 3, cap: 3 })).toMatch(/^Tried 3 times without finishing \(limit 3\)\./);
    expect(unblock({ reason: 'stuck' })).toMatch(/^Stopped making progress/);
    expect(unblock({ reason: 'refinement_budget', cap: 4 })).toMatch(/^Used all 4 revision rounds/);
  });
});

describe('eyebrow, title and technical details', () => {
  it('names kinds in human words, never wire ids (U15)', () => {
    expect(kindLabel(fixture('dec-004a-wallclock'))).toBe('Goal ran out of time');
    expect(kindLabel(fixture('dec-004d-delete'))).toBe('Deletion request');
    for (const d of MOCK_PENDING_DECISIONS) expect(kindLabel(d)).not.toMatch(/_|tier|unblock/);
  });

  it('uses the full goal title, never the daemon’s truncated quoted headline (U2)', () => {
    const d = fixture('dec-004a-wallclock');
    expect(d.headline.endsWith('…')).toBe(true);
    expect(decisionTitle(d)).toBe(d.goal_title);
    expect(decisionTitle(fixture('dec-003'))).toBe('Pick a path: where to keep your decision history');
  });

  it('falls back to detail and the row ids; prefers the daemon technical block', () => {
    const d = fixture('dec-004a-wallclock');
    const fallback = technicalDetails(d);
    expect(fallback.detail).toBe(d.detail);
    expect(fallback.ids.map(i => i.label)).toEqual(['Decision', 'Goal', 'Project']);
    const daemon = technicalDetails({ ...d, technical: { detail: 'exhaust_and_park', ids: { run: 'r-1' }, last_error: 'timeout' } });
    expect(daemon).toMatchObject({ detail: 'exhaust_and_park', lastError: 'timeout' });
    expect(daemon.ids[0]).toEqual({ label: 'run', value: 'r-1' });
  });

  it('formats durations and ages for humans (R3)', () => {
    expect(humanDuration(240)).toBe('4 min');
    expect(humanDuration(45)).toBe('45 sec');
    expect(humanDuration(3900)).toBe('1 hr 5 min');
    const now = Date.parse('2026-09-25T12:00:00Z');
    expect(ageLabel('2026-09-25T11:47:00Z', now)).toBe('13 min ago');
    expect(ageLabel('2026-09-25T07:00:00Z', now)).toBe('5 hr ago');
  });
});
