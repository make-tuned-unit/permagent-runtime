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
