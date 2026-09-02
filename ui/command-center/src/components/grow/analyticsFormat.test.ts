/**
 * Drain freshness: the staleness rule behind "a stale figure must never read
 * as a quiet day". The label is relative ("drained 2m ago") and the stale
 * flag flips past a day — the UI's warning tint hangs off it. The threshold
 * has to clear the poller's once-a-day interval, or the tint would be on
 * almost permanently and stop meaning anything.
 */

import { describe, expect, it } from 'vitest';
import { drainFreshness, DRAIN_STALE_MS } from './analyticsFormat';

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
