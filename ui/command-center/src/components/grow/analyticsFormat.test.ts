/**
 * Drain freshness: the staleness rule behind "a stale figure must never read
 * as a quiet day". The label is relative ("drained 2m ago") and the stale
 * flag flips at one hour — the UI's warning tint hangs off it.
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

  it('flips stale past one hour — the warning-tint threshold', () => {
    expect(drainFreshness(at(DRAIN_STALE_MS - 1), NOW)?.stale).toBe(false);
    expect(drainFreshness(at(DRAIN_STALE_MS + 60_000), NOW)).toEqual({
      label: 'drained 1h ago',
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
