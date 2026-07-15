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

/**
 * Build the meter model from the latest {@link TokenState}. Shows exactly ONE
 * cost-ish number (the running session $) to avoid the "Usage $ vs Spending %"
 * confusion; the cache figure is explicitly a *saving*, not a second spend.
 */
export function formatCostMeter(tokens: TokenState | null): CostMeterModel {
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

  return { cost, segments, ariaLabel: aria.join(', ') };
}
