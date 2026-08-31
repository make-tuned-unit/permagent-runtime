/**
 * Freshness — the app's one answer to "how old is this?".
 *
 * Two surfaces had grown their own: the Brain rendered a memory's age from a
 * clamped 0..1 scene scalar (so 91 days and 3 years read identically), and Home
 * composed its own "Updated 2m ago · reconnecting" caption with a private
 * minutes/hours ladder. Same question, two vocabularies, two ideas of what old
 * means — and each new surface that needed a timestamp invented a third.
 *
 * This module owns the vocabulary; `<AsOf>` owns how it looks. Between them
 * there is exactly one place to change what staleness says and one place to
 * change what it looks like.
 *
 * Two granularities, because the two honest resolutions are genuinely
 * different: a polled figure ages in minutes, a memory ages in days. Below a
 * day they diverge; above one, they share the same ladder.
 */

import { useEffect, useState } from 'react';

export type FreshnessTone =
  /** Current enough to state plainly. */
  | 'live'
  /** Old enough that its age is part of what it says. */
  | 'stale'
  /** No usable date at all — never guess one. */
  | 'unknown';

export interface Freshness {
  /** How old it is, in the app's one relative-time vocabulary. */
  label: string;
  tone: FreshnessTone;
  /** `tone !== 'live'` — the boolean most call sites actually branch on. */
  stale: boolean;
  /** Epoch millis, or null when there was nothing usable to read. */
  at: number | null;
  /** The precise moment, for a tooltip. Null when unknown. */
  exact: string | null;
}

export interface FreshnessOptions {
  /**
   * `clock` for things that are polled (a dashboard, a feed): resolves in
   * moments/minutes/hours before handing over to the day scale.
   * `calendar` for things that are dated (a memory, a note): days, weeks,
   * months, years — "today", never "0m ago".
   */
  granularity?: 'clock' | 'calendar';
  /**
   * How old it may get before its age is worth noticing. Default: never — an
   * old date is not automatically a problem, and only the caller knows the
   * shelf life of its own subject. `0` means "stale the moment it is not
   * live", which is what a surface whose poll is failing wants.
   */
  staleAfterMs?: number;
  /** What to say when there is no usable date. Deliberately not blank. */
  unknownLabel?: string;
}

/**
 * Anything that can name a moment → epoch millis, or null.
 *
 * A bare "YYYY-MM-DD HH:MM:SS" is the daemon's wire shape and is UTC; the
 * browser would otherwise read it as local time and shift every reading by the
 * timezone offset — an hours-wide lie on a surface whose subject is time.
 */
export function parseTimestamp(value: number | string | Date | null | undefined): number | null {
  if (value == null || value === '') return null;
  if (value instanceof Date) {
    const ms = value.getTime();
    return Number.isNaN(ms) ? null : ms;
  }
  if (typeof value === 'number') return Number.isFinite(value) ? value : null;
  const hasZone = value.endsWith('Z') || /[+-]\d\d:?\d\d$/.test(value);
  const norm = hasZone ? value : `${value.replace(' ', 'T')}Z`;
  const ms = new Date(norm).getTime();
  return Number.isNaN(ms) ? null : ms;
}

const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;

/** The canonical reading. Pure — pass `now` to pin the clock. */
export function freshnessOf(
  asOf: number | string | Date | null | undefined,
  options: FreshnessOptions = {},
  now = Date.now(),
): Freshness {
  const { granularity = 'clock', staleAfterMs = Number.POSITIVE_INFINITY, unknownLabel = 'date unknown' } = options;

  const at = parseTimestamp(asOf);
  if (at == null) {
    // Missing is not "old" and is certainly not "today" — say which it is.
    return { label: unknownLabel, tone: 'unknown', stale: true, at: null, exact: null };
  }

  // A timestamp in the future is a clock skew or a forward-dated import. Read
  // it as now rather than inventing a countdown.
  const age = Math.max(0, now - at);
  const stale = age >= staleAfterMs;

  return {
    label: granularity === 'clock' ? clockLabel(age) : calendarLabel(age),
    tone: stale ? 'stale' : 'live',
    stale,
    at,
    exact: new Date(at).toLocaleString(),
  };
}

/** Sub-day resolution, then the calendar ladder. Never "0m ago". */
function clockLabel(age: number): string {
  if (age < MINUTE) return 'moments ago';
  if (age < HOUR) return `${Math.floor(age / MINUTE)}m ago`;
  if (age < DAY) return `${Math.floor(age / HOUR)}h ago`;
  const days = Math.floor(age / DAY);
  if (days < 30) return `${days}d ago`;
  return calendarLabel(age);
}

/** Day resolution and coarser. Coarse where coarse is honest (nobody needs
 *  "17 days ago"), but it never stops counting — that clamp was the bug. */
function calendarLabel(age: number): string {
  const days = Math.floor(age / DAY);
  if (days === 0) return 'today';
  if (days === 1) return 'yesterday';
  if (days < 7) return `${days} days ago`;
  if (days < 30) {
    const weeks = Math.round(days / 7);
    return weeks <= 1 ? 'last week' : `${weeks} weeks ago`;
  }
  if (days < 365) {
    const months = Math.round(days / 30.44);
    if (months < 12) return months <= 1 ? 'last month' : `${months} months ago`;
  }
  const years = Math.max(1, Math.round(days / 365.25));
  return years === 1 ? 'last year' : `${years} years ago`;
}

export interface UseFreshnessOptions extends FreshnessOptions {
  /** How often the label re-reads the clock, so an open screen's "2m ago"
   *  doesn't sit at 2m for an hour. */
  intervalMs?: number;
  /** Pin the clock (tests, stories). Set, and the label stops ticking. */
  now?: number;
}

/** `freshnessOf`, kept current while the screen is open. */
export function useFreshness(
  asOf: number | string | Date | null | undefined,
  options: UseFreshnessOptions = {},
): Freshness {
  const { intervalMs = 30_000, now: pinned, ...rest } = options;
  const [tick, setTick] = useState(() => pinned ?? Date.now());

  useEffect(() => {
    if (pinned != null) return;
    setTick(Date.now());
    const id = setInterval(() => setTick(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [pinned, intervalMs, asOf]);

  return freshnessOf(asOf, rest, pinned ?? tick);
}
