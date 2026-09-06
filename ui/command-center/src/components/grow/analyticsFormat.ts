// Formatting helpers for the Grow analytics panels. Pure functions, unit
// tested — the staleness rule ("a stale figure must never read as a quiet
// day", the botsExcluded pattern) lives here rather than inline in JSX.

/** A drain older than this is stale enough to warn about. */
export const DRAIN_STALE_MS = 60 * 60 * 1000;

export interface DrainFreshness {
  /** e.g. "drained 2m ago" */
  label: string;
  /** True when the last successful drain is over an hour old — the figures on
   *  screen may be arbitrarily behind, and must not read as a quiet day. */
  stale: boolean;
}

/** Freshness of the drain loop, from the stats payload's `lastDrainAt`.
 *  Returns null when the timestamp is absent or unparsable — an honest gap,
 *  rendered as nothing rather than as "fresh". */
export function drainFreshness(
  lastDrainAt: string | null | undefined,
  now: Date = new Date(),
): DrainFreshness | null {
  if (!lastDrainAt) return null;
  const at = new Date(lastDrainAt).getTime();
  if (Number.isNaN(at)) return null;
  const ageMs = Math.max(0, now.getTime() - at);
  const mins = Math.floor(ageMs / 60_000);
  const label = mins < 1
    ? 'drained just now'
    : mins < 60
      ? `drained ${mins}m ago`
      : mins < 48 * 60
        ? `drained ${Math.floor(mins / 60)}h ago`
        : `drained ${Math.floor(mins / (24 * 60))}d ago`;
  return { label, stale: ageMs > DRAIN_STALE_MS };
}

/** A complete UTC day in the first-party analytics graph. */
export interface DailyAnalyticsPoint {
  day: string;
  pageviews: number;
  visitors: number;
}

const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * Fill a backend `byDay` response into a stable, contiguous series.
 *
 * The daemon's day keys are UTC dates. Building the range with local
 * `Date#setDate` and then serialising it can cross a day boundary in a
 * negative/positive offset timezone (and during DST), which makes the graph
 * appear to lose or gain a day. The range is therefore anchored at UTC
 * midnight and advanced by whole UTC days.
 */
export function dailyAnalyticsSeries(
  byDay: readonly DailyAnalyticsPoint[],
  periodDays: number,
  endDate: Date = new Date(),
): DailyAnalyticsPoint[] {
  const count = Math.max(0, Math.floor(periodDays));
  if (count === 0) return [];

  const endUtc = Date.UTC(
    endDate.getUTCFullYear(),
    endDate.getUTCMonth(),
    endDate.getUTCDate(),
  );
  const existing = new Map(byDay.map((point) => [point.day, point]));

  return Array.from({ length: count }, (_, index) => {
    const day = new Date(endUtc - (count - 1 - index) * DAY_MS)
      .toISOString()
      .slice(0, 10);
    return existing.get(day) ?? { day, pageviews: 0, visitors: 0 };
  });
}

export interface Trendline {
  slope: number;
  intercept: number;
  start: number;
  end: number;
}

/** Least-squares trendline for a time series, with x = zero-based day. */
export function linearTrendline(values: readonly number[]): Trendline | null {
  if (values.length === 0) return null;
  if (values.length === 1) {
    const value = Number.isFinite(values[0]) ? values[0] : 0;
    return { slope: 0, intercept: value, start: value, end: value };
  }

  const safeValues = values.map((value) => (Number.isFinite(value) ? value : 0));
  const xMean = (safeValues.length - 1) / 2;
  const yMean = safeValues.reduce((sum, value) => sum + value, 0) / safeValues.length;
  const denominator = safeValues.reduce((sum, _, index) => sum + (index - xMean) ** 2, 0);
  const slope = denominator === 0
    ? 0
    : safeValues.reduce((sum, value, index) => sum + (index - xMean) * (value - yMean), 0) / denominator;
  const intercept = yMean - slope * xMean;
  return {
    slope,
    intercept,
    start: intercept,
    end: intercept + slope * (safeValues.length - 1),
  };
}
