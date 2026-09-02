/**
 * The app's one answer to "how old is this?".
 *
 * Two renderings of staleness had grown up independently — the Brain's memory
 * age and Home's "the figures stopped being refreshed" caption — with their own
 * vocabularies and their own idea of what old means. This is the shared one.
 * The cases below are the union of what those two sites promised, so folding
 * them onto this must not change a single sentence a user reads.
 */

import { describe, expect, it } from 'vitest';
import { freshnessOf, parseTimestamp } from './useFreshness';

const NOW = Date.parse('2026-08-31T12:00:00Z');
const daysAgo = (d: number) => NOW - d * 86_400_000;
const minutesAgo = (m: number) => NOW - m * 60_000;

describe('freshnessOf — calendar granularity (dated things: memories, notes)', () => {
  const label = (at: number) => freshnessOf(at, { granularity: 'calendar' }, NOW).label;

  it('counts in days, then weeks, months and years', () => {
    expect(label(daysAgo(0))).toBe('today');
    expect(label(daysAgo(1))).toBe('yesterday');
    expect(label(daysAgo(4))).toBe('4 days ago');
    expect(label(daysAgo(9))).toBe('last week');
    expect(label(daysAgo(21))).toBe('3 weeks ago');
    expect(label(daysAgo(45))).toBe('last month');
    expect(label(daysAgo(240))).toBe('8 months ago');
    expect(label(daysAgo(400))).toBe('last year');
  });

  it('never stops counting — the whole point of having a scale', () => {
    // A clamped scale used to render 91 days and 3 years with the same words.
    expect(label(daysAgo(91))).not.toBe(label(daysAgo(3 * 365)));
    expect(label(daysAgo(3 * 365))).toBe('3 years ago');
    expect(label(daysAgo(20 * 365))).toBe('20 years ago');
  });

  it('reads a future timestamp as now rather than inventing a countdown', () => {
    expect(label(NOW + 86_400_000)).toBe('today');
  });
});

describe('freshnessOf — clock granularity (polled things: dashboards, feeds)', () => {
  const label = (at: number) => freshnessOf(at, { granularity: 'clock' }, NOW).label;

  it('resolves below a day, then hands over to the calendar scale', () => {
    expect(label(NOW - 4_000)).toBe('moments ago');
    expect(label(minutesAgo(2))).toBe('2m ago');
    expect(label(minutesAgo(30))).toBe('30m ago');
    expect(label(minutesAgo(150))).toBe('2h ago');
    expect(label(minutesAgo(60 * 30))).toBe('1d ago');
    expect(label(daysAgo(60))).toBe('2 months ago');
  });

  it('never says "0m ago"', () => {
    expect(label(NOW)).toBe('moments ago');
    expect(label(NOW - 59_000)).toBe('moments ago');
  });
});

describe('freshnessOf — staleness', () => {
  it('is not stale by default, at any age', () => {
    const f = freshnessOf(daysAgo(400), { granularity: 'calendar' }, NOW);
    expect(f.stale).toBe(false);
    expect(f.tone).toBe('live');
  });

  it('turns stale past the threshold the caller sets', () => {
    const opts = { granularity: 'calendar' as const, staleAfterMs: 90 * 86_400_000 };
    expect(freshnessOf(daysAgo(10), opts, NOW).stale).toBe(false);
    expect(freshnessOf(daysAgo(89), opts, NOW).stale).toBe(false);
    expect(freshnessOf(daysAgo(120), opts, NOW).stale).toBe(true);
    expect(freshnessOf(daysAgo(120), opts, NOW).tone).toBe('stale');
  });

  it('treats staleAfterMs 0 as "the moment it is not live"', () => {
    expect(freshnessOf(NOW, { staleAfterMs: 0 }, NOW).stale).toBe(true);
  });
});

describe('freshnessOf — no usable date', () => {
  it('says so instead of guessing, and is neither fresh nor old', () => {
    const f = freshnessOf(null, {}, NOW);
    expect(f.label).toBe('date unknown');
    expect(f.tone).toBe('unknown');
    expect(f.stale).toBe(true);
    expect(f.exact).toBeNull();
  });

  it('lets the caller name what "nothing yet" is called', () => {
    expect(freshnessOf(undefined, { unknownLabel: 'never' }, NOW).label).toBe('never');
    expect(freshnessOf('not-a-date', { unknownLabel: 'never' }, NOW).label).toBe('never');
  });
});

describe('freshnessOf — exact', () => {
  it('carries the precise timestamp for a tooltip', () => {
    const f = freshnessOf(daysAgo(3), {}, NOW);
    expect(f.at).toBe(daysAgo(3));
    expect(f.exact).toBe(new Date(daysAgo(3)).toLocaleString());
  });
});

describe('parseTimestamp', () => {
  it('accepts epoch millis, Date, and ISO strings', () => {
    expect(parseTimestamp(NOW)).toBe(NOW);
    expect(parseTimestamp(new Date(NOW))).toBe(NOW);
    expect(parseTimestamp('2026-08-31T12:00:00Z')).toBe(NOW);
  });

  it('reads a bare "date time" as UTC — the daemon wire shape, not local time', () => {
    expect(parseTimestamp('2026-08-31 12:00:00')).toBe(NOW);
  });

  it('returns null for anything unusable rather than a fabricated date', () => {
    expect(parseTimestamp(null)).toBeNull();
    expect(parseTimestamp(undefined)).toBeNull();
    expect(parseTimestamp('')).toBeNull();
    expect(parseTimestamp('not-a-date')).toBeNull();
    expect(parseTimestamp(Number.NaN)).toBeNull();
  });
});
