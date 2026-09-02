/**
 * One count, one set of words (J3).
 *
 * Five surfaces render the same `/api/decisions` queue. The DETAIL surface was
 * already correctly shared — `DecisionInbox`, and `ApprovalsStrip`'s own
 * comment says it "never forks that surface". What was duplicated is the
 * summary chrome above it: two independent builds saying different things about
 * one number ("3 pending approvals" in Settings, "Nova needs 3 answers" on
 * Home), which is how a user comes to wonder whether they are two queues.
 */

import { describe, expect, it } from 'vitest';
import { summarizeDecisions } from './summary';
import type { DecisionsResponse } from './types';

const resp = (over: Partial<DecisionsResponse> = {}): DecisionsResponse => ({
  decisions: [],
  total_pending: 0,
  handled_count: 0,
  goals_in_flight: 0,
  goals_needing_attention: 0,
  oldest_pending_at: null,
  ...over,
} as DecisionsResponse);

describe('decision summary', () => {
  it('says it is still checking rather than showing a zero it has not read', () => {
    const s = summarizeDecisions(null, 'Nova');
    expect(s.loading).toBe(true);
    expect(s.headline).toBe('Checking with Nova…');
    expect(s.allClear).toBe(false);
  });

  it('names the agent and the number in one sentence, singular and plural', () => {
    expect(summarizeDecisions(resp({ total_pending: 1 }), 'Nova').headline)
      .toBe('Nova needs 1 answer');
    expect(summarizeDecisions(resp({ total_pending: 3 }), 'Nova').headline)
      .toBe('Nova needs 3 answers');
  });

  it('is not all-clear while a parked goal is waiting on the user', () => {
    const s = summarizeDecisions(resp({ total_pending: 0, goals_needing_attention: 1 }), 'Nova');
    expect(s.allClear).toBe(false);
    expect(s.attentionLabel).toBe('1 parked goal needs your attention');
  });

  it('is all-clear only when nothing is pending and nothing is parked', () => {
    const s = summarizeDecisions(resp({ goals_in_flight: 2 }), 'Nova');
    expect(s.allClear).toBe(true);
    expect(s.allClearLabel).toBe('All clear — 2 goals in flight');
  });

  it('carries the age of the oldest item so both placements can show it', () => {
    const s = summarizeDecisions(
      resp({ total_pending: 2, oldest_pending_at: new Date(Date.now() - 7_200_000).toISOString() }),
      'Nova',
    );
    expect(s.oldestLabel).toMatch(/^oldest waiting /);
  });

  it('has no oldest line when nothing is waiting', () => {
    expect(summarizeDecisions(resp(), 'Nova').oldestLabel).toBeNull();
  });

  it('mentions the overnight routine work only when there was some', () => {
    expect(summarizeDecisions(resp(), 'Nova').handledLabel).toBeNull();
    expect(summarizeDecisions(resp({ handled_count: 4 }), 'Nova').handledLabel)
      .toBe('Nova handled 4 routine items overnight');
  });

  it('lets Home override the goal count with the shared live-goals source', () => {
    const s = summarizeDecisions(resp({ goals_in_flight: 2 }), 'Nova', 5);
    expect(s.allClearLabel).toBe('All clear — 5 goals in flight');
  });
});
