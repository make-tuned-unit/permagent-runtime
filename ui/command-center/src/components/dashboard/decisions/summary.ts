/**
 * The Decision Inbox's summary, in one place (J3: Home is canonical).
 *
 * Five surfaces render this queue. The DETAIL surface has always been correctly
 * shared — every one of them opens `DecisionInbox`, and `ApprovalsStrip`'s own
 * comment says it "never forks that surface". What was duplicated was the
 * chrome ABOVE it: Home said "Nova needs 3 answers", Settings → Autonomy said
 * "Pending approvals: 3", each computed from the same payload by its own hand.
 * Two different sentences about one number is how a user comes to suspect there
 * are two queues.
 *
 * So the words live here, and every placement imports them. Home renders them
 * as its card (the canonical rendering); Settings renders the same sentence as
 * a one-line reference that hands you back to Home.
 *
 * Nothing here decides layout — only what is true and what it is called.
 */

import { formatAge } from './format';
import type { DecisionsResponse } from './types';

export interface DecisionSummary {
  /** Nothing has been read yet. NOT the same as a zero. */
  loading: boolean;
  count: number;
  handled: number;
  attention: number;
  goals: number;
  oldestAt: string | null;
  /** Nothing pending AND nothing parked. A parked goal is not "all clear". */
  allClear: boolean;
  /** The one sentence for the count, wherever it is rendered. */
  headline: string;
  allClearLabel: string;
  /** Null when there is nothing waiting to be old. */
  oldestLabel: string | null;
  attentionLabel: string | null;
  handledLabel: string | null;
}

/**
 * @param data      the decisions payload, or null while the first read is out.
 * @param agentName what the user calls their agent.
 * @param activeGoals Home passes the shared live-goals count so its cards agree
 *                    with each other; everything else uses the payload's own.
 */
export function summarizeDecisions(
  data: DecisionsResponse | null,
  agentName: string,
  activeGoals?: number,
): DecisionSummary {
  const count = data?.total_pending ?? 0;
  const handled = data?.handled_count ?? 0;
  const attention = data?.goals_needing_attention ?? 0;
  const goals = activeGoals ?? data?.goals_in_flight ?? 0;
  const oldestAt = data?.oldest_pending_at ?? null;
  const loading = data === null;

  return {
    loading,
    count,
    handled,
    attention,
    goals,
    oldestAt,
    // A parked goal waiting on the user is not "all clear" (wave-1 item 1).
    allClear: !loading && count === 0 && attention === 0,
    headline: loading
      ? `Checking with ${agentName}…`
      : `${agentName} needs ${count} answer${count === 1 ? '' : 's'}`,
    allClearLabel: `All clear — ${goals} goal${goals === 1 ? '' : 's'} in flight`,
    oldestLabel: count > 0 && oldestAt ? `oldest waiting ${formatAge(oldestAt)}` : null,
    attentionLabel: attention > 0
      ? `${attention} parked goal${attention === 1 ? '' : 's'} need${attention === 1 ? 's' : ''} your attention`
      : null,
    handledLabel: handled > 0
      ? `${agentName} handled ${handled} routine item${handled === 1 ? '' : 's'} overnight`
      : null,
  };
}
