import { describe, expect, it } from 'vitest';

import { evidenceTargetFor } from './decisionEvidence';
import { MOCK_PENDING_DECISIONS } from './mockDecisions';
import type { Decision } from './types';

function fixture(id: string): Decision {
  const found = MOCK_PENDING_DECISIONS.find(d => d.id === id);
  if (!found) throw new Error(`fixture ${id} not found`);
  return found;
}

describe('evidenceTargetFor', () => {
  it('resolves goal-bound decisions to dispatch evidence coordinates', () => {
    expect(evidenceTargetFor(fixture('dec-001'))).toEqual({
      kind: 'dispatch',
      projectId: 'proj-permagent',
      goalId: 'goal-queue-saving',
      label: 'Open evidence for Decision queue saving',
    });
  });

  it('uses council_action payload session id instead of any "latest" fallback', () => {
    const council: Decision = {
      ...fixture('dec-001'),
      kind: 'council_action',
      goal_id: null,
      project_id: null,
      payload: {
        session_id: 'council-sess-9',
        title: 'Run this',
      },
    };
    expect(evidenceTargetFor(council)).toEqual({
      kind: 'council_report',
      councilSessionId: 'council-sess-9',
      label: 'Open council report',
    });
  });

  it('returns none for council_action when payload session id is absent', () => {
    const council: Decision = {
      ...fixture('dec-001'),
      kind: 'council_action',
      goal_id: null,
      project_id: null,
      payload: { title: 'Run this' },
    };
    expect(evidenceTargetFor(council)).toEqual({
      kind: 'none',
      reason: 'council_action payload has no council session id.',
    });
  });

  it('resolves S2N unblock to the pause episode target', () => {
    const scheduled: Decision = {
      ...fixture('dec-004'),
      payload: {
        target: { type: 'scheduled_job', job_id: 'job-42' },
        pause_episode_id: 'ep-3',
        reason: 'consecutive_failure_limit',
      },
      goal_id: null,
      project_id: null,
    };
    expect(evidenceTargetFor(scheduled)).toEqual({
      kind: 'pause_episode',
      jobId: 'job-42',
      episodeId: 'ep-3',
      label: 'Open pause episode evidence',
    });
  });

  it('returns budget evidence target for reservation-gate choices', () => {
    const budgetChoice: Decision = {
      ...fixture('dec-003'),
      payload: {
        question: 'Task budget authorization needs approval',
        options: [{ id: 'increment', label: 'Add to the budget and continue' }],
        reservation_gate: {
          scope: 'task',
          settled_usd: 5,
          held_usd: 1,
          requested_usd: 0.7,
          ceiling_usd: 6,
        },
      },
      goal_id: null,
      project_id: null,
    };
    const target = evidenceTargetFor(budgetChoice);
    expect(target.kind).toBe('budget');
    if (target.kind === 'budget') {
      expect(target.detail).toContain('task budget gate');
      expect(target.detail).toContain('$5.00');
      expect(target.detail).toContain('$0.70');
    }
  });
});
