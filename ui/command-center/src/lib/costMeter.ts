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

import type { SSEEvent, TokenState } from './api';

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
  turnUsd: number;
  sessionUsd: number;
  todayUsd: number;
  totalTokens: number;
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
): CostMeterModel {
  if (coding) {
    const sessionCost = fmtUsd(coding.sessionUsd);
    // A fail-closed estimate rendered as a plain "$" reads as a bill the
    // harness actually charged — the `~` is the difference between "this is
    // what it cost" and "this is the worst case we're guarding against".
    const cost = coding.estimated ? `~${sessionCost}` : sessionCost;
    const todayCost = fmtUsd(coding.todayUsd);

    const segments: string[] = [
      `${fmtTokens(coding.totalTokens)} tokens`,
      `+${fmtUsd(coding.turnUsd)} this turn`,
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
