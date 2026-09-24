/**
 * What the notifications tray may say about the Decision Inbox.
 *
 * QA 2026-09-24: the tray read "Nothing needs you — all quiet." with 23
 * decisions pending, each one marked "needs you". The tray's list is only the
 * live `/events` pings received since this window mounted (lib/notifications.ts
 * drops the replay buffer on purpose), so a decision created before launch, or
 * one the router sent to the digest channel instead of `in_app`, never became
 * a tray item. The empty state then spoke for a queue it had never read.
 *
 * The count now comes from the same payload Home reads (`useDecisions` →
 * `summarizeDecisions`), and "all quiet" is only allowed when that payload
 * positively says so. Loading and a failed read are not zero.
 */

import type { DecisionSummary } from '../dashboard/decisions/summary';

export type TrayDecisionState =
  /** The first read is still out: say nothing about the queue yet. */
  | { kind: 'checking' }
  /** The read failed with nothing on screen: the queue is unknown, not empty. */
  | { kind: 'unknown' }
  /** Something is waiting on the user. */
  | { kind: 'waiting'; headline: string; detail: string | null }
  /** The payload says nothing is pending and nothing is parked. */
  | { kind: 'clear' };

export function trayDecisionState(summary: DecisionSummary, error: boolean): TrayDecisionState {
  if (error) return { kind: 'unknown' };
  if (summary.loading) return { kind: 'checking' };
  if (summary.allClear) return { kind: 'clear' };
  // Home's own sentence for the count (summary.ts: one number, one sentence,
  // wherever it is rendered) — a parked goal alone still counts as waiting.
  const headline = summary.count > 0 ? summary.headline : (summary.attentionLabel ?? summary.headline);
  const detail = summary.count > 0
    ? [summary.oldestLabel, summary.attentionLabel].filter(Boolean).join(' · ') || null
    : null;
  return { kind: 'waiting', headline, detail };
}

/** "All quiet" is only true when there are no tray items AND the queue is clear. */
export function trayIsQuiet(itemCount: number, state: TrayDecisionState): boolean {
  return itemCount === 0 && state.kind === 'clear';
}
