import { describe, expect, it } from 'vitest';

import { MOCK_PENDING_DECISIONS } from './mockDecisions';
import { effectSummaryFor } from './decisionEffects';
import type { Decision } from './types';

function byId(id: string): Decision {
  const found = MOCK_PENDING_DECISIONS.find(d => d.id === id);
  if (!found) throw new Error(`missing fixture ${id}`);
  return found;
}

describe('effectSummaryFor', () => {
  it('covers every kind represented in mockDecisions fixtures', () => {
    const fixtures = ['dec-001', 'dec-002-conflict', 'dec-003', 'dec-004', 'dec-004b-draft'];
    for (const id of fixtures) {
      const summary = effectSummaryFor(byId(id));
      expect(summary.known).toBe(true);
      if (summary.known) expect(summary.lines.length).toBeGreaterThan(0);
    }
  });

  it('returns record-only for unknown kinds', () => {
    const unknown: Decision = { ...byId('dec-001'), kind: 'new_kind' };
    expect(effectSummaryFor(unknown)).toEqual({
      known: false,
      text: 'Records your answer; no automatic effect',
    });
  });

  it('renders scheduled-job unblock (S2N) with resume vs keep-paused lines', () => {
    const scheduled: Decision = {
      ...byId('dec-004'),
      payload: {
        target: { type: 'scheduled_job', job_id: 'job-42' },
        reason: 'consecutive_failure_limit',
        pause_episode_id: 'ep-7',
        answers: [
          { id: 'approve', label: 'Resume schedule' },
          { id: 'reject', label: 'Keep paused' },
        ],
      },
    };
    const summary = effectSummaryFor(scheduled);
    expect(summary.known).toBe(true);
    if (summary.known) {
      expect(summary.lines[0]).toContain(
        'resumes the job and resets consecutive failures and retry count to 0',
      );
      expect(summary.lines[1]).toContain('keep the scheduled job paused');
      expect(summary.missing).toBeUndefined();
    }
  });

  it('renders scheduled-job unblock (S2N) by answer id when answers are reversed', () => {
    const scheduled: Decision = {
      ...byId('dec-004'),
      payload: {
        target: { type: 'scheduled_job', job_id: 'job-42' },
        reason: 'consecutive_failure_limit',
        pause_episode_id: 'ep-7',
        answers: [
          { id: 'reject', label: 'Keep paused' },
          { id: 'approve', label: 'Resume schedule' },
        ],
      },
    };
    const summary = effectSummaryFor(scheduled);
    expect(summary.known).toBe(true);
    if (summary.known) {
      expect(summary.lines[0]).toContain('Resume schedule');
      expect(summary.lines[0]).toContain(
        'resumes the job and resets consecutive failures and retry count to 0',
      );
      expect(summary.lines[1]).toContain('Keep paused');
      expect(summary.lines[1]).toContain('keep the scheduled job paused and change nothing else');
      expect(summary.missing).toBeUndefined();
    }
  });

  it('lists missing scheduled-job details when the payload is incomplete', () => {
    const scheduled: Decision = {
      ...byId('dec-004'),
      payload: {
        target: { type: 'scheduled_job' },
        reason: 'consecutive_failure_limit',
      },
    };
    const summary = effectSummaryFor(scheduled);
    expect(summary.known).toBe(true);
    if (summary.known) {
      expect(summary.missing).toContain('scheduled job target');
      expect(summary.missing).toContain('pause episode id');
    }
  });

  it('renders K1 convene-budget choice options with budget context', () => {
    const budgetChoice: Decision = {
      ...byId('dec-003'),
      payload: {
        question: 'Task budget authorization needs approval',
        options: [
          { id: 'increment', label: 'Add to the budget and continue' },
          { id: 'complete', label: 'Treat as complete at current quality' },
        ],
        default: 'increment',
        reservation_gate: {
          scope: 'task',
          settled_usd: 5,
          held_usd: 1,
          requested_usd: 0.7,
          ceiling_usd: 6,
          requesting_session_id: 'sess-1',
        },
      },
    };
    const summary = effectSummaryFor(budgetChoice);
    expect(summary.known).toBe(true);
    if (summary.known) {
      expect(summary.lines[0]).toMatch(/^1\./);
      expect(summary.lines[1]).toMatch(/^2\./);
      expect(summary.lines.join('\n')).toContain('Budget context (task)');
      expect(summary.lines.join('\n')).toContain('does not itself promise an automatic convene');
    }
  });
});
