/**
 * Drain freshness: the staleness rule behind "a stale figure must never read
 * as a quiet day". The label is relative ("drained 2m ago") and the stale
 * flag flips past a day — the UI's warning tint hangs off it. The threshold
 * has to clear the poller's once-a-day interval, or the tint would be on
 * almost permanently and stop meaning anything.
 */

import { describe, expect, it } from 'vitest';
import {
  dailyAnalyticsSeries,
  drainFreshness,
  DRAIN_STALE_MS,
  evenTileColumns,
  linearTrendline,
} from './analyticsFormat';

const NOW = new Date('2026-08-10T12:00:00Z');
const at = (msAgo: number) => new Date(NOW.getTime() - msAgo).toISOString();

describe('drainFreshness', () => {
  it('is null when there is no timestamp — an honest gap, not "fresh"', () => {
    expect(drainFreshness(null, NOW)).toBeNull();
    expect(drainFreshness(undefined, NOW)).toBeNull();
    expect(drainFreshness('not-a-date', NOW)).toBeNull();
  });

  it('labels a recent drain in minutes and does not warn', () => {
    expect(drainFreshness(at(30_000), NOW)).toEqual({ label: 'drained just now', stale: false });
    expect(drainFreshness(at(2 * 60_000), NOW)).toEqual({ label: 'drained 2m ago', stale: false });
    expect(drainFreshness(at(59 * 60_000), NOW)).toEqual({ label: 'drained 59m ago', stale: false });
  });

  it('does not warn on a drain that is merely a day old — that is the schedule', () => {
    // The poller polls once a day, so these are healthy, not stale. Under the
    // old one-hour threshold every one of these warned.
    expect(drainFreshness(at(2 * 60 * 60_000), NOW)).toEqual({ label: 'drained 2h ago', stale: false });
    expect(drainFreshness(at(23 * 60 * 60_000), NOW)?.stale).toBe(false);
    expect(drainFreshness(at(DRAIN_STALE_MS - 1), NOW)?.stale).toBe(false);
  });

  it('flips stale once a whole poll cycle has been missed', () => {
    expect(DRAIN_STALE_MS).toBeGreaterThan(24 * 60 * 60_000);
    expect(drainFreshness(at(DRAIN_STALE_MS + 60_000), NOW)).toEqual({
      label: 'drained 26h ago',
      stale: true,
    });
    expect(drainFreshness(at(3 * 24 * 60 * 60_000), NOW)).toEqual({
      label: 'drained 3d ago',
      stale: true,
    });
  });

  it('treats a clock-skewed future timestamp as just now, never negative', () => {
    expect(drainFreshness(at(-5 * 60_000), NOW)).toEqual({ label: 'drained just now', stale: false });
  });
});

describe('dailyAnalyticsSeries', () => {
  it('fills missing days using UTC dates, independent of local timezone', () => {
    const series = dailyAnalyticsSeries(
      [{ day: '2026-03-07', pageviews: 4, visitors: 2 }],
      3,
      new Date('2026-03-09T00:15:00Z'),
    );

    expect(series.map((day) => day.day)).toEqual(['2026-03-07', '2026-03-08', '2026-03-09']);
    expect(series.map((day) => day.pageviews)).toEqual([4, 0, 0]);
  });

  it('does not let a fractional or negative period produce an invalid range', () => {
    expect(dailyAnalyticsSeries([], 2.9, new Date('2026-01-03T12:00:00Z'))).toHaveLength(2);
    expect(dailyAnalyticsSeries([], -1, new Date('2026-01-03T12:00:00Z'))).toEqual([]);
  });
});

describe('linearTrendline', () => {
  it('captures a rising direction across the series', () => {
    expect(linearTrendline([1, 3, 5])).toEqual({ slope: 2, intercept: 1, start: 1, end: 5 });
  });

  it('returns a flat line for a single point and null for no points', () => {
    expect(linearTrendline([7])).toEqual({ slope: 0, intercept: 7, start: 7, end: 7 });
    expect(linearTrendline([])).toBeNull();
  });
});

describe('evenTileColumns (QA 2026-09-24: six tiles in five columns)', () => {
  it('caps the track count so six tiles land 3×2 or 2×3, never 5+1', () => {
    const cols = evenTileColumns(3, 140, 12);
    // Each track is at least a third of the row (minus the two gaps), so no
    // fourth or fifth column can form however wide the panel is.
    expect(cols).toBe('repeat(auto-fit, minmax(max(140px, calc((100% - 24px) / 3)), 1fr))');
    expect(cols).not.toContain('110px');
  });
});
