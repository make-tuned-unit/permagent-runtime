/**
 * The pure rules of the action → verify → measure loop: what a verdict is
 * called, how a change was confirmed, when a window closes, and what a metric
 * value reads as.
 *
 * Split out of GrowView.tsx (R9). No JSX and no fetches — every one of these is
 * a fact about the backend's own vocabulary (metrics.rs, growth_actions.rs), so
 * they live where the cards, the rail and the verify control can all reach the
 * same copy. Two copies of `categoryColor` drifting apart is the kind of
 * difference a reader would read as meaning.
 */

import type { ThemeColors } from '../../styles/tokens';
import type { ActionIdentity, ActionOutcome } from './growTypes';

/** The closed set an action may pre-register against (metrics.rs:41-47). Kept
 *  in the same order and spelling the backend parses, so a select can only ever
 *  produce a value `TargetMetric::parse` accepts. */
export const TARGET_METRICS: { value: string; label: string }[] = [
  { value: 'pageviews', label: 'pageviews' },
  { value: 'sessions', label: 'sessions' },
  { value: 'aeo_visits', label: 'answer-engine visits' },
  { value: 'bounce_rate', label: 'bounce rate' },
];

/**
 * Verdict → label and colour.
 *
 * `inconclusive` is the one case that must NOT be tinted like a problem. The
 * proposal makes it the expected outcome at this traffic — "≲100 views/week —
 * per-project verdicts stay `inconclusive` essentially always" — and states the
 * rule directly: "It must be the visually neutral default, not a sad grey
 * state, or there is pressure to manufacture verdicts"
 * (docs/proposals/grow-action-outcome-loop.md:46-48, :173-175). So it borrows
 * the same `textMuted` the body copy uses rather than `danger` or a dimmer grey
 * than the settled `no_effect`.
 */
export function verdictMeta(verdict: string, colors: ThemeColors): { label: string; color: string } {
  switch (verdict) {
    case 'helped': return { label: 'Helped', color: colors.success };
    case 'hindered': return { label: 'Hindered', color: colors.danger };
    case 'no_effect': return { label: 'No detectable change', color: colors.textDim };
    case 'confounded': return { label: 'Overlapped another change', color: colors.textDim };
    default: return { label: 'Not enough data to say', color: colors.textMuted };
  }
}

/**
 * How the change was confirmed, in words that say what was actually checked.
 *
 * The proposal's requirement, and the reason `verified_by` is a column at all:
 * "'Verified from a commit' and 'you told me so' are different claims and must
 * not look identical" (proposal:107-109). `checked` drives the styling apart as
 * well as the wording — self-attestation gets a dashed rule and the warning
 * tint, so the two are distinguishable at a glance and not only on a careful
 * read.
 */
export function verifiedByMeta(how: string | null | undefined): { label: string; checked: boolean } {
  switch (how) {
    case 'git': return { label: 'Verified from a commit in this project’s repo', checked: true };
    case 'content': return { label: 'Verified on the live page', checked: true };
    case 'event': return { label: 'Verified from a traffic source that was not there before', checked: true };
    case 'self': return { label: 'You told me it landed — your word, not a check', checked: false };
    default: return { label: 'Not verified', checked: false };
  }
}

/**
 * When a window can first be judged.
 *
 * The pivot is the day AFTER verification, and the window completes once `days`
 * have fully elapsed from it (`pivot_date` at metrics.rs:156-157,
 * `window_is_complete` at :191-192). Rendering the date is what stops an empty
 * outcome list reading as "it found nothing" when the truth is "it is not due
 * yet".
 */
export function windowDueAt(verifiedAt: string | null, days: number): Date | null {
  if (!verifiedAt) return null;
  const at = new Date(verifiedAt);
  if (Number.isNaN(at.getTime())) return null;
  const due = new Date(at);
  due.setUTCDate(due.getUTCDate() + 1 + days);
  return due;
}

/** The measurement windows, in the order and spelling the sweep uses
 *  (`metrics.rs` WINDOW_DAYS). The Tracking view walks all three; anything that
 *  shows only the first tells the user a 28-day verdict is not coming. */
export const WINDOW_DAYS = [7, 14, 28];
/** The shortest window is 7 days (metrics.rs WINDOW_DAYS), the longest 28. */
export const FIRST_WINDOW_DAYS = WINDOW_DAYS[0];
export const FINAL_WINDOW_DAYS = WINDOW_DAYS[WINDOW_DAYS.length - 1];

/** Where one measurement window has got to.
 *
 *  `judged` — the sweep has written an outcome for it.
 *  `due`    — the window has fully elapsed but no outcome exists yet (the sweep
 *             runs nightly, so this is a real and honest state, not an error).
 *  `open`   — still accumulating; `dueAt` says when it closes.
 */
export type WindowState = 'judged' | 'due' | 'open';

export interface WindowProgress {
  days: number;
  state: WindowState;
  dueAt: Date | null;
  outcome: ActionOutcome | null;
}

/**
 * How far through the 7/14/28-day windows an action is.
 *
 * Derived here rather than sent by the server because every input is already on
 * the wire — `verifiedAt` and the outcomes — and a second source for the same
 * fact is a second thing that can disagree with the sweep. The boundary is the
 * same one `metrics::window_is_complete` uses: the pivot is the day AFTER
 * verification, and the window closes once `days` have fully elapsed from it.
 */
export function windowProgress(
  identity: ActionIdentity,
  now: Date = new Date(),
): WindowProgress[] {
  return WINDOW_DAYS.map((days) => {
    const outcome = identity.outcomes.find((o) => o.windowDays === days) ?? null;
    const dueAt = windowDueAt(identity.verifiedAt, days);
    const state: WindowState = outcome
      ? 'judged'
      : dueAt && dueAt.getTime() <= now.getTime()
        ? 'due'
        : 'open';
    return { days, state, dueAt, outcome };
  });
}

/** A metric value in the units the metric is actually in. A bounce rate is a
 *  proportion in [0,1] and rendering it as "0.99" beside a pageview count reads
 *  as a broken number rather than as 99%. */
export function metricValue(metric: string, value: number): string {
  if (metric === 'bounce_rate') return `${(value * 100).toFixed(0)}%`;
  return value.toLocaleString(undefined, { maximumFractionDigits: 1 });
}

/**
 * Category → tint, at module scope so the archived shelf and the active board
 * colour the same category identically. Two copies drifted apart is the kind of
 * difference a reader would read as meaning.
 */
export function categoryColor(category: string, colors: ThemeColors): string {
  const map: Record<string, string> = {
    conversion: colors.cyan,
    retention: colors.success,
    churn: colors.danger,
    ux: colors.purple,
    acquisition: colors.warning ?? colors.cyan,
    measurement: colors.textMuted,
    content: colors.cyan,
    seo: colors.success,
    aeo: colors.purple,
  };
  return map[category] ?? colors.textDim;
}

/**
 * The states an action may be filed away from.
 *
 * `suggested` is absent deliberately, and `reject_pointless_archive`
 * (growth_actions.rs) refuses it on the server for the same reason: archiving
 * is what releases an action's text for re-proposal, so filing away something
 * that was never acted on would hand the identical advice straight back on the
 * next review. Dismissal is the control for advice the user does not want, and
 * it keeps the text off the board.
 */
export const ARCHIVABLE = ['done', 'verified', 'measuring', 'judged', 'dismissed'];

/** Which list a card is in. See `ActionCard`'s `lane` prop. */
export type ActionLane = 'actions' | 'tracking' | 'shelf';
