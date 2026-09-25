/**
 * What one "Agents at work" row says under its name.
 *
 * `/api/runs` (crates/goose-server/src/routes/runs.rs) sends a schedule row's
 * detail as the raw croner expression ("0 0 6 * * 1-5"), "every Ns", or
 * "once at <RFC3339>". It carries no last error and no failure count, so those
 * come from the schedule list the Automate view already holds (`/schedule/list`),
 * joined here by id.
 */

import { cronToEnglish } from '../../lib/schedule-format';

/** The /schedule/list facts the roster joins onto a schedule row. */
export interface RosterScheduleFacts {
  id: string;
  cron?: string;
  tz?: string | null;
  paused?: boolean;
  last_error?: string | null;
  consecutive_failures?: number;
  /** The daemon's own "is this running right now" — see `reconcileRosterRuns`.
   *  Left `undefined` (not `false`) when the caller doesn't know, so a test
   *  fixture built before this field existed cannot be silently downgraded. */
  currently_running?: boolean;
  process_start_time?: string | null;
  display_name?: string | null;
}

export interface RosterRunItem {
  kind: 'worker' | 'schedule' | 'session';
  id: string;
  name: string;
  status: 'working' | 'idle' | 'error' | 'missed' | 'paused';
  detail: string | null;
  last_activity: string | null;
  started_at: string | null;
  interruptible: boolean;
}

export interface RosterDetail {
  /** The plain-words line, or the detail as sent when it has no words. */
  words: string | null;
  /** The raw expression, shown small beside the words; null when `words` IS it. */
  raw: string | null;
}

function intervalWords(secs: number): string {
  const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? '' : 's'}`;
  if (secs % 86400 === 0) return plural(secs / 86400, 'day');
  if (secs % 3600 === 0) return plural(secs / 3600, 'hour');
  if (secs % 60 === 0) return plural(secs / 60, 'minute');
  return plural(secs, 'second');
}

export function rosterDetail(
  kind: 'worker' | 'schedule' | 'session',
  detail: string | null,
  facts?: RosterScheduleFacts,
): RosterDetail {
  if (kind !== 'schedule') return { words: detail, raw: null };
  const cron = facts?.cron || detail;
  if (!cron) return { words: null, raw: null };

  const every = cron.match(/^every (\d+)s$/);
  if (every) return { words: `Every ${intervalWords(Number(every[1]))}`, raw: null };
  const once = cron.match(/^once at (.+)$/);
  if (once) {
    const d = new Date(once[1]);
    return { words: Number.isNaN(d.getTime()) ? cron : `Once on ${d.toLocaleString()}`, raw: null };
  }

  const words = cronToEnglish(cron);
  if (words === cron) return { words: cron, raw: null };
  return { words: facts?.tz ? `${words} (${facts.tz})` : words, raw: cron };
}

/**
 * R5: "Agents at work" (`/api/runs`, a best-effort aggregate) and "Running
 * Now" (`/schedule/list`'s own `currently_running`) are two independent
 * polls of the very same fact for a schedule, and can disagree for a beat —
 * or entirely, when the aggregate's row for a fresh run hasn't landed yet.
 * QA 2026-09-25: the roster said "all resting" directly above a card reading
 * "RUNNING NOW — Downloads sweep". `/schedule/list`'s `currently_running` is
 * the scheduler's own state, so for any schedule it is the one truth this
 * reconciles the roster to before render, rather than keeping two answers to
 * one question on screen at once.
 */
export function reconcileRosterRuns(
  runs: RosterRunItem[],
  schedules: RosterScheduleFacts[],
): RosterRunItem[] {
  const byId = new Map(schedules.map(s => [s.id, s] as const));
  const seen = new Set<string>();
  const reconciled = runs.map(r => {
    if (r.kind !== 'schedule') return r;
    const facts = byId.get(r.id);
    if (!facts || typeof facts.currently_running !== 'boolean') return r;
    seen.add(r.id);
    if (facts.currently_running) {
      return r.status === 'working' ? r : { ...r, status: 'working' as const, started_at: facts.process_start_time ?? r.started_at };
    }
    // The schedule list says this one is NOT running — `/api/runs` cannot
    // outrank that for the one binary fact it also claims to know.
    return r.status === 'working' ? { ...r, status: 'idle' as const } : r;
  });
  // Running per the schedule list, but missing from `/api/runs` entirely —
  // the run started between polls, or the aggregate simply missed the row.
  const missing: RosterRunItem[] = schedules
    .filter(s => s.currently_running === true && !seen.has(s.id))
    .map(s => ({
      kind: 'schedule' as const, id: s.id, name: s.display_name || s.id, status: 'working' as const,
      detail: null, last_activity: null, started_at: s.process_start_time ?? null, interruptible: true,
    }));
  return missing.length > 0 ? [...reconciled, ...missing] : reconciled;
}

/** The line explaining a paused or failing schedule, or null when there is nothing to say. */
export function rosterProblem(status: string, facts?: RosterScheduleFacts): string | null {
  const err = facts?.last_error?.trim() || null;
  const fails = facts?.consecutive_failures ?? 0;
  if (status === 'paused') {
    const why = fails > 0 ? `Paused after ${fails} failed run${fails === 1 ? '' : 's'}` : null;
    if (why && err) return `${why}: ${err}`;
    return why ?? (err ? `Last error: ${err}` : null);
  }
  if (status === 'error' && err) return `Last error: ${err}`;
  return null;
}
