/**
 * Build cost-meter model — the pure logic behind the always-on Build statusline.
 *
 * Split out from the React component (and shared with the store) so the SSE →
 * meter path is testable in the repo's node-env vitest with no DOM: the store's
 * `handleSessionEvent` extracts the frame's cost via {@link costFromFrame}, and
 * the component renders {@link formatCostMeter}. A test that runs
 * `formatCostMeter(costFromFrame(frame))` therefore exercises the identical code
 * that runs in production — real frame in, rendered `$` out, nothing stubbed.
 */

import type {
  BillingEvidence,
  BudgetCapTriplet,
  BudgetProjection,
  BudgetScopeProjection,
  ProjectionBand,
  ProjectionCompleteness,
  SSEEvent,
  TokenState,
} from './api';

const PROJECTION_VERSION = 'budget-projection.v1' as const;

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function finiteOrNull(value: unknown): number | null | undefined {
  if (value === null) return null;
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return undefined;
  return value;
}

function nullableString(value: unknown): string | null | undefined {
  if (value === null) return null;
  return typeof value === 'string' ? value : undefined;
}

function requiredString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null;
}

function completeness(value: unknown): ProjectionCompleteness | null {
  return value === 'complete' || value === 'partial' || value === 'unknown' ? value : null;
}

function band(value: unknown): ProjectionBand | null | undefined {
  if (value === null) return null;
  return value === 'ok' || value === 'soft' || value === 'gate'
    || value === 'hard' || value === 'unknown' ? value : undefined;
}

function parseCap(value: unknown): BudgetCapTriplet | null {
  const cap = record(value);
  if (!cap) return null;
  const softUsd = finiteOrNull(cap.softUsd);
  const gateUsd = finiteOrNull(cap.gateUsd);
  const hardUsd = finiteOrNull(cap.hardUsd);
  const source = requiredString(cap.source);
  if (softUsd === undefined || gateUsd === undefined || hardUsd === undefined || !source) return null;
  // A partially present cap is unavailable, never a zero ceiling.
  if (softUsd === null || gateUsd === null || hardUsd === null) {
    if (softUsd !== null || gateUsd !== null || hardUsd !== null) return null;
  } else if (softUsd > gateUsd || gateUsd > hardUsd) {
    return null;
  }
  return { softUsd, gateUsd, hardUsd, source };
}

function parseScope(value: unknown): BudgetScopeProjection | null {
  const scope = record(value);
  if (!scope) return null;
  const cap = parseCap(scope.cap);
  const settledUsd = finiteOrNull(scope.settledUsd);
  const heldUsd = finiteOrNull(scope.heldUsd);
  const unknownUsd = finiteOrNull(scope.unknownUsd);
  const effectiveUsedUsd = finiteOrNull(scope.effectiveUsedUsd);
  const remainingUsd = finiteOrNull(scope.remainingUsd);
  const parsedBand = band(scope.band);
  const parsedCompleteness = completeness(scope.completeness);
  const error = nullableString(scope.error);
  if (!cap || settledUsd === undefined || heldUsd === undefined || unknownUsd === undefined
    || effectiveUsedUsd === undefined || remainingUsd === undefined || parsedBand === undefined
    || !parsedCompleteness || error === undefined) return null;
  return {
    cap, settledUsd, heldUsd, unknownUsd, effectiveUsedUsd, remainingUsd,
    band: parsedBand, completeness: parsedCompleteness, error,
  };
}

function parseBilling(value: unknown): BillingEvidence | null {
  const billing = record(value);
  if (!billing) return null;
  const billingClass = nullableString(billing.billingClass);
  const provider = nullableString(billing.provider);
  const model = nullableString(billing.model);
  const callId = nullableString(billing.callId);
  const isEstimated = billing.isEstimated === null
    ? null
    : typeof billing.isEstimated === 'boolean' ? billing.isEstimated : undefined;
  const observedAt = nullableString(billing.observedAt);
  const source = requiredString(billing.source);
  if (billingClass === undefined || provider === undefined || model === undefined
    || callId === undefined || isEstimated === undefined || observedAt === undefined || !source) return null;
  if (observedAt !== null && !Number.isFinite(Date.parse(observedAt))) return null;
  return { billingClass, provider, model, callId, isEstimated, observedAt, source };
}

/** Strict runtime guard for the daemon's versioned canonical projection.
 * Invalid identity, version, null semantics, or non-finite numbers return null
 * so callers can render unavailable instead of fabricating a zero. */
export function parseBudgetProjection(value: unknown): BudgetProjection | null {
  const projection = record(value);
  if (!projection || projection.provenance === undefined) return null;
  const taskId = !Object.prototype.hasOwnProperty.call(projection, 'taskId')
    ? undefined
    : projection.taskId === null
      ? null
      : typeof projection.taskId === 'string' && projection.taskId.trim().length > 0
        ? projection.taskId
        : undefined;
  const rootSessionId = requiredString(projection.rootSessionId);
  const task = parseScope(projection.task);
  const session = parseScope(projection.session);
  const taskBilling = parseBilling(projection.taskBilling);
  const sessionBilling = parseBilling(projection.sessionBilling);
  const provenance = record(projection.provenance);
  const version = provenance ? provenance.version : undefined;
  const asOf = provenance ? provenance.asOf : undefined;
  const provenanceCompleteness = provenance ? completeness(provenance.completeness) : null;
  const sources = provenance?.sources;
  const provenanceError = nullableString(provenance?.error);
  if (taskId === undefined || !rootSessionId || !task || !session || !taskBilling || !sessionBilling
    || version !== PROJECTION_VERSION || typeof asOf !== 'string'
    || !Number.isFinite(Date.parse(asOf)) || !provenanceCompleteness
    || !Array.isArray(sources) || !sources.every(source => typeof source === 'string' && source.length > 0)
    || provenanceError === undefined) return null;
  return {
    taskId, rootSessionId, task, session, taskBilling, sessionBilling,
    provenance: {
      version: PROJECTION_VERSION,
      asOf,
      completeness: provenanceCompleteness,
      sources: [...sources],
      error: provenanceError,
    },
  };
}

/**
 * Pull the live token + cost state off any SSE frame that carries one (Message /
 * Finish). Returns `null` for frames without `token_state` (Ping, Error, …) so
 * the meter holds its last value rather than flickering to zero.
 */
export function costFromFrame(data: SSEEvent): TokenState | null {
  const ts = (data as { token_state?: TokenState }).token_state;
  return ts ?? null;
}

/** Compact USD: cents by default, extra precision for sub-cent amounts. */
export function fmtUsd(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '$0.00';
  if (n < 0.01) return `$${n.toFixed(4)}`;
  return `$${n.toFixed(2)}`;
}

/** Compact token count: 47k, 1.2M. */
export function fmtTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0';
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return `${Math.round(n)}`;
}

/**
 * Direct-child subagent spend rolled into a parent session
 * (`GET /api/sessions/{id}/cost`). When `count > 0`, the statusline appends
 * `incl. N subagents $X`.
 */
export interface SubagentCostIncl {
  count: number;
  totalUsd: number;
}

/**
 * Spend announcement from the CODING HARNESS's own session (`permagent run
 * --recipe permagent-coding --interactive`, running as a PTY subprocess in the
 * Build terminal). This is NOT the browser chat session `TokenState` above —
 * the harness mints its own session id in its own process and writes correct
 * cost-ledger rows under THAT id, which the browser chat SSE stream never
 * carries. Arrives via the daemon's `session_spend_changed` bus frame
 * (livenessSync.ts), snake_case on the wire, camelCased here at the boundary.
 */
export interface CodingSpend {
  sessionId: string;
  turnUsd: number | null;
  sessionUsd: number | null;
  todayUsd: number | null;
  totalTokens: number | null;
  provider: string | null;
  model: string | null;
  workingDir: string | null;
  /** True when the last call had NO published price and was billed at the
   *  fail-closed worst case (the most expensive rate in the registry) —
   *  deliberately over-stated so a spend cap fires early. Rendering this
   *  as a plain bill would present a safety margin as a fact, so the meter
   *  must show it as an estimate, not a number to trust literally. */
  estimated: boolean;
  /** True on the session's closing announcement — the total is final, not
   *  merely the value between two turns. */
  finalTurn: boolean;
  /** Canonical projection, when supplied by B5.3. Legacy events omit this. */
  budget?: BudgetProjection;
  /** Set when hydration observed an active run but its canonical projection
   * was unavailable/unknown. This suppresses chat-account substitution. */
  budgetStatus?: 'available' | 'unknown' | 'unavailable';
}

/** Rendered statusline model. `cost` is THE authoritative number; `segments`
 *  are supporting context. Kept as plain strings so the component is a trivial,
 *  faithful renderer of this model (which is what the wiring test asserts). */
export interface CostMeterModel {
  /** The one authoritative running-session figure, e.g. "$0.42". */
  cost: string;
  /** Ordered supporting segments, e.g. ["47k↑ 12k↓", "cache saved $0.28", "31% ctx", "claude-…"]. */
  segments: string[];
  /** Screen-reader summary. */
  ariaLabel: string;
}

export type CodingHarnessHydration = 'initial' | 'active' | 'none' | 'unavailable';

function fmtNullableUsd(value: number | null): string {
  return value === null ? 'unavailable' : fmtUsd(value);
}

/**
 * Is this scope's "remaining" figure something the user can take at face value?
 *
 * The user, looking at the Build bar that read "session remaining $47.91 ·
 * session unknown $1.56 · session budget unknown": "I am unsure what all of
 * this is". The bar stated an exact remaining figure while, in the same line,
 * admitting it did not know the budget state. Remaining is the hard cap minus
 * used, and the daemon's "used" (budget_projection.rs) counts `unknown_usd` —
 * reservations for provider calls whose outcome could not be confirmed, which
 * may or may not have been charged — and settled rows that were priced by
 * estimate (`is_estimated`, which is what makes a scope "partial" when there
 * are no unconfirmed holds). Either way the subtraction inherits that doubt,
 * so such a figure is shown qualified, never flat.
 */
function remainingIsConfirmed(scope: BudgetScopeProjection): boolean {
  return scope.completeness === 'complete'
    && scope.band !== 'unknown'
    && !hasUnconfirmedHolds(scope);
}

function hasUnconfirmedHolds(scope: BudgetScopeProjection): boolean {
  return scope.unknownUsd !== null && scope.unknownUsd > 0;
}

/**
 * Why the remaining figure is only approximate, in the daemon's own terms.
 * The daemon marks a scope "partial" when it has unconfirmed holds or settled
 * calls priced by estimate; with no holds, "partial" can only mean the latter.
 */
function unconfirmedRemainingReason(scope: BudgetScopeProjection): string {
  if (hasUnconfirmedHolds(scope)) {
    return `${fmtUsd(scope.unknownUsd!)} is held for calls whose charge could not be confirmed`;
  }
  if (scope.completeness === 'partial') return 'some calls were priced by estimate';
  return 'the budget status is unknown';
}

/**
 * The status-bar segments for the harness session's budget: at most three
 * short phrases a non-expert can read in one glance — how much is held for
 * calls whose charge could not be confirmed, how much is reserved for calls
 * still running, and what is left. The ledger vocabulary (settled / held / effective / band) stays out of
 * the bar; every figure is still spelled out in {@link formatBudgetDetail},
 * which the tooltip and the screen-reader label carry.
 */
function appendSessionBudgetSegments(
  segments: string[],
  aria: string[],
  scope: BudgetScopeProjection,
): void {
  if (hasUnconfirmedHolds(scope)) {
    // Was "session unknown $1.56": accurate to the ledger column name and
    // meaningless to the person paying. It is the sum of reservations in state
    // 'unknown' (budget_projection.rs): budget held for provider calls whose
    // outcome could not be confirmed — a stream that failed to set up, usage
    // that never arrived, a settlement write that failed, or a lease that
    // expired while the call "may already have been accepted remotely"
    // (session_manager.rs). It is NOT spend with no published price; those
    // calls are settled rows priced by estimate and already inside "settled".
    // So it is worded as unconfirmed, never as unpriced.
    const unconfirmed = fmtUsd(scope.unknownUsd!);
    segments.push(`${unconfirmed} unconfirmed`);
    aria.push(`${unconfirmed} of that is held for calls whose charge could not be confirmed and may or may not be billed`);
  }
  if (scope.heldUsd !== null && scope.heldUsd > 0) {
    const held = fmtUsd(scope.heldUsd);
    segments.push(`${held} pending`);
    aria.push(`${held} reserved for calls still in progress`);
  }
  if (scope.remainingUsd === null) {
    segments.push('budget left unknown');
    aria.push('remaining budget unknown');
  } else if (remainingIsConfirmed(scope)) {
    const left = fmtUsd(scope.remainingUsd);
    segments.push(`${left} left`);
    aria.push(`${left} of budget left`);
  } else {
    const left = fmtUsd(scope.remainingUsd);
    segments.push(`~${left} left (unconfirmed)`);
    aria.push(`about ${left} of budget left, not confirmed because ${unconfirmedRemainingReason(scope)}`);
  }
}

/** One line per fact, for the tooltip: every figure the bar summarises, in
 *  words, so condensing the bar loses no information. */
export function formatBudgetDetail(budget: BudgetProjection): string {
  const usd = (value: number | null) => (value === null ? 'unavailable' : fmtUsd(value));
  const scopeLines = (name: string, scope: BudgetScopeProjection): string[] => {
    const cap = scope.cap;
    const remaining = scope.remainingUsd === null
      ? 'unknown'
      : remainingIsConfirmed(scope)
        ? usd(scope.remainingUsd)
        : `about ${usd(scope.remainingUsd)} (unconfirmed)`;
    // "settled" includes calls priced by estimate (no published price), so it
    // is never called "priced"; when the scope is partial with no unconfirmed
    // holds, estimated prices are the only reason the daemon gives for that.
    const settledNote = scope.completeness === 'partial' && !hasUnconfirmedHolds(scope)
      ? ' (some calls used estimated prices)'
      : '';
    return [
      `${name} spent ${usd(scope.effectiveUsedUsd)}`,
      `  settled ${usd(scope.settledUsd)}${settledNote}`,
      `  unconfirmed (held for calls whose charge could not be confirmed) ${usd(scope.unknownUsd)}`,
      `  pending (reserved for calls in progress) ${usd(scope.heldUsd)}`,
      `  budget left ${remaining}`,
      `  limits: warn ${usd(cap.softUsd)} / pause ${usd(cap.gateUsd)} / stop ${usd(cap.hardUsd)}`,
      `  budget status ${scope.band ?? 'no limit'}, figures ${scope.completeness}`,
    ];
  };
  const billing = (b: BillingEvidence) => {
    const text = [b.billingClass, b.provider, b.model].filter(Boolean).join(' / ') || 'unavailable';
    return b.isEstimated === true ? `${text} (estimated)` : text;
  };
  return [
    ...scopeLines('Session', budget.session),
    `  billing ${billing(budget.sessionBilling)}`,
    ...scopeLines('Task', budget.task),
    `  billing ${billing(budget.taskBilling)}`,
    `Sources ${budget.provenance.sources.join(', ') || 'unavailable'}`,
    `As of ${budget.provenance.asOf} (${budget.provenance.version})`,
    ...(budget.provenance.error ? [`Error ${budget.provenance.error}`] : []),
  ].join('\n');
}

/**
 * Segments that must read as a caution rather than ordinary context: a figure
 * that is not a confirmed bill (an estimate, or a hold whose charge is unknown). The component colours these with the theme's
 * warning colour; keeping the rule here keeps it next to the words it matches.
 */
export function isCautionSegment(seg: string): boolean {
  return seg === 'estimated — no published price'
    || seg === 'estimated'
    || seg.endsWith(' unconfirmed')
    || seg.endsWith('(unconfirmed)');
}

function appendSubagentSegment(
  segments: string[],
  aria: string[],
  subagents: SubagentCostIncl | null | undefined,
): void {
  if (!subagents || subagents.count <= 0) return;
  const n = subagents.count;
  const label = n === 1 ? 'subagent' : 'subagents';
  const dollars = fmtUsd(subagents.totalUsd);
  const seg = `incl. ${n} ${label} ${dollars}`;
  segments.push(seg);
  aria.push(seg);
}

/**
 * Build the meter model from the latest {@link TokenState}, optional coding
 * harness {@link CodingSpend}, and optional child-subagent rollup.
 *
 * When `coding` is non-null it is AUTHORITATIVE: it is the Build tab's own PTY
 * session, and `tokens` (the browser chat session) is a different account
 * entirely. Subagent suffix is appended whenever children exist on either
 * source.
 */
export function formatCostMeter(
  tokens: TokenState | null,
  coding: CodingSpend | null = null,
  subagents: SubagentCostIncl | null = null,
  harnessHydration: CodingHarnessHydration = 'initial',
): CostMeterModel {
  if (!coding && harnessHydration === 'unavailable') {
    return {
      cost: '—',
      segments: ['Budget unavailable'],
      ariaLabel: 'Budget unavailable; harness spend is not currently readable',
    };
  }
  if (!coding && !tokens && harnessHydration === 'none') {
    return {
      cost: '—',
      segments: ['No active harness'],
      ariaLabel: 'No active coding harness',
    };
  }
  if (coding) {
    if (coding.budget) {
      const sessionScope = coding.budget.session;
      const sessionCost = fmtNullableUsd(sessionScope.effectiveUsedUsd);
      const cost = sessionScope.effectiveUsedUsd === null ? '—' : fmtUsd(sessionScope.effectiveUsedUsd);
      const segments: string[] = [];
      const aria: string[] = [`Session cost ${sessionCost}`];
      appendSessionBudgetSegments(segments, aria, sessionScope);
      // The task figure only earns a segment when it says something the
      // session figure does not; its full breakdown is in the tooltip.
      const taskUsed = coding.budget.task.effectiveUsedUsd;
      if (taskUsed !== null && taskUsed !== sessionScope.effectiveUsedUsd) {
        segments.push(`this task ${fmtUsd(taskUsed)}`);
        aria.push(`this task ${fmtUsd(taskUsed)}`);
      }
      if (coding.budget.sessionBilling.isEstimated === true) {
        segments.push('estimated');
        aria.push('session billing is estimated, not a final bill');
      }
      if (coding.budget.provenance.error) {
        segments.push('budget figures incomplete');
        aria.push(`budget projection error ${coding.budget.provenance.error}`);
      }
      if (harnessHydration === 'unavailable' || coding.budgetStatus === 'unavailable') {
        segments.push('last known — live figures unavailable');
        aria.push('live budget figures unavailable; showing last known harness spend');
      }
      if (coding.finalTurn) {
        segments.push('session ended');
        aria.push('session ended');
      }
      appendSubagentSegment(segments, aria, subagents);
      return { cost, segments, ariaLabel: aria.join(', ') };
    }

    if (coding.budgetStatus === 'unknown' || coding.budgetStatus === 'unavailable') {
      const label = coding.budgetStatus === 'unknown' ? 'Budget unknown' : 'Budget unavailable';
      const segments = [label];
      const aria = [label];
      appendSubagentSegment(segments, aria, subagents);
      return { cost: '—', segments, ariaLabel: aria.join(', ') };
    }

    const sessionCost = coding.sessionUsd === null ? 'unavailable' : fmtUsd(coding.sessionUsd);
    // A fail-closed estimate rendered as a plain "$" reads as a bill the
    // harness actually charged — the `~` is the difference between "this is
    // what it cost" and "this is the worst case we're guarding against".
    const cost = coding.estimated ? `~${sessionCost}` : sessionCost;
    const todayCost = coding.todayUsd === null ? 'unavailable' : fmtUsd(coding.todayUsd);

    const segments: string[] = [
      `${coding.totalTokens === null ? 'unavailable' : fmtTokens(coding.totalTokens)} tokens`,
      `+${coding.turnUsd === null ? 'unavailable' : fmtUsd(coding.turnUsd)} this turn`,
      `today ${todayCost}`,
    ];
    const aria: string[] = [`Session cost ${sessionCost}`, `today ${todayCost}`];

    if (coding.model) {
      segments.push(coding.model);
      aria.push(`model ${coding.model}`);
    }
    if (coding.estimated) {
      // The segment text, not just the `~`, so the reason is on screen —
      // a squiggle alone is easy to miss or mistake for a rounding mark.
      segments.push('estimated — no published price');
      aria.push('this figure is an estimate, not a final bill');
    }
    if (coding.finalTurn) {
      segments.push('session ended');
      aria.push('session ended');
    }
    appendSubagentSegment(segments, aria, subagents);

    return { cost, segments, ariaLabel: aria.join(', ') };
  }

  if (!tokens) {
    return { cost: '$0.00', segments: [], ariaLabel: 'No cost recorded yet' };
  }

  const cost = fmtUsd(tokens.accumulatedCostUsd);
  const segments: string[] = [];
  const aria: string[] = [`Session cost ${cost}`];

  segments.push(
    `${fmtTokens(tokens.accumulatedInputTokens)}↑ ${fmtTokens(tokens.accumulatedOutputTokens)}↓`,
  );
  aria.push(
    `${tokens.accumulatedInputTokens} input tokens, ${tokens.accumulatedOutputTokens} output tokens`,
  );

  if (tokens.cacheSavingsUsd > 0) {
    const saved = fmtUsd(tokens.cacheSavingsUsd);
    segments.push(`cache saved ${saved}`);
    aria.push(`cache saved ${saved}`);
  }

  if (tokens.contextPercent != null && Number.isFinite(tokens.contextPercent)) {
    const pct = Math.round(tokens.contextPercent);
    segments.push(`${pct}% ctx`);
    aria.push(`${pct} percent of context used`);
  }

  if (tokens.model) {
    segments.push(tokens.model);
    aria.push(`model ${tokens.model}`);
  }

  appendSubagentSegment(segments, aria, subagents);

  return { cost, segments, ariaLabel: aria.join(', ') };
}
