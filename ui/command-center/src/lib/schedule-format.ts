/**
 * Schedules in plain words.
 *
 * The daemon's scheduler is croner, whose expressions have SIX fields:
 * `sec min hour dom mon dow` ("0 0 6 * * 1-5"). This helper used to read the
 * first five as `min hour dom mon dow`, so every real schedule shifted one
 * field: the seconds became the minute and the HOUR became the day of the
 * month. "0 0 6 * * 1-5" (weekdays at 6) read "Day 6 of every month at 12 AM"
 * on the Automate cards, and the roster, which never called it, showed the raw
 * string (QA 2026-09-24). Five-field input still works (seconds = 0).
 *
 * The Automate cards and the "Agents at work" roster both read this one
 * helper (`scheduleSummary`, `rosterDetail`) — there is no second parser.
 *
 * What it knows: fixed times on every day, weekdays, weekends or a day list or
 * range; every N minutes / hours; hourly at a minute; a day of the month.
 * Anything else comes back as the raw expression, unchanged — a wrong sentence
 * is worse than the honest string.
 */

const DAY_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const DAY_PLURAL = ['Sundays', 'Mondays', 'Tuesdays', 'Wednesdays', 'Thursdays', 'Fridays', 'Saturdays'];
const DAY_NAMES: Record<string, number> = { SUN: 0, MON: 1, TUE: 2, WED: 3, THU: 4, FRI: 5, SAT: 6 };
/** Read order: the week as people say it, Monday first. */
const WEEK_ORDER = [1, 2, 3, 4, 5, 6, 0];

const NUM = /^\d+$/;

function dayNumber(token: string): number | null {
  const t = token.toUpperCase();
  if (t in DAY_NAMES) return DAY_NAMES[t];
  if (!NUM.test(t)) return null;
  const n = Number(t);
  if (n < 0 || n > 7) return null;
  return n === 7 ? 0 : n;
}

/** The set of weekdays a dow field names, or null if it isn't a plain list/range. */
export function parseDayOfWeek(field: string): Set<number> | null {
  if (field === '*' || field === '?') return new Set([0, 1, 2, 3, 4, 5, 6]);
  const days = new Set<number>();
  for (const part of field.split(',')) {
    const range = part.split('-');
    if (range.length === 1) {
      const d = dayNumber(part);
      if (d === null) return null;
      days.add(d);
    } else if (range.length === 2) {
      const a = dayNumber(range[0]);
      // 7 at the END of a range means Sunday-as-7 ("5-7" = Fri..Sun).
      const bRaw = range[1] === '7' ? 7 : dayNumber(range[1]);
      if (a === null || bRaw === null || bRaw < a) return null;
      for (let d = a; d <= bRaw; d++) days.add(d % 7);
    } else {
      return null;
    }
  }
  return days.size > 0 ? days : null;
}

function joinWords(items: string[]): string {
  if (items.length <= 1) return items.join('');
  return `${items.slice(0, -1).join(', ')} & ${items[items.length - 1]}`;
}

function daysPhrase(days: Set<number>): string {
  const has = (list: number[]) => list.length === days.size && list.every(d => days.has(d));
  if (days.size === 7) return 'Daily';
  if (has([1, 2, 3, 4, 5])) return 'Weekdays';
  if (has([0, 6])) return 'Weekends';
  const ordered = WEEK_ORDER.filter(d => days.has(d));
  if (ordered.length === 1) return DAY_PLURAL[ordered[0]];
  return joinWords(ordered.map(d => DAY_SHORT[d]));
}

export function formatClock(h: number, m: number): string {
  const period = h >= 12 ? 'PM' : 'AM';
  const hh = h % 12 === 0 ? 12 : h % 12;
  return `${hh}:${String(m).padStart(2, '0')} ${period}`;
}

function ordinal(n: number): string {
  const rem100 = n % 100;
  if (rem100 >= 11 && rem100 <= 13) return `${n}th`;
  return `${n}${({ 1: 'st', 2: 'nd', 3: 'rd' } as Record<number, string>)[n % 10] ?? 'th'}`;
}

/** Normalise to [sec, min, hour, dom, mon, dow]; null when it isn't one of
 *  the shapes below. croner itself takes 5 or 6 fields; a Quartz-style 7th
 *  (year) field is accepted only as `*`, since a year filter has no words here. */
function fields(cron: string): string[] | null {
  const parts = cron.trim().split(/\s+/);
  if (parts.length === 5) return ['0', ...parts];
  if (parts.length === 6) return parts;
  if (parts.length === 7 && parts[6] === '*') return parts.slice(0, 6);
  return null;
}

/** Convert a croner (6-field) or classic (5-field) expression to plain words.
 *  Unrecognised shapes return the input unchanged. */
export function cronToEnglish(cron: string): string {
  const f = fields(cron);
  if (!f) return cron;
  const [sec, min, hour, dom, mon, dow] = f;
  // Sub-minute schedules and month filters are rare enough to leave raw.
  if (sec !== '0' || mon !== '*') return cron;

  const everyDay = dom === '*' && (dow === '*' || dow === '?');

  const minStep = min.match(/^\*\/(\d+)$/);
  if ((minStep || min === '*') && hour === '*' && everyDay) {
    const n = minStep ? Number(minStep[1]) : 1;
    return n === 1 ? 'Every minute' : `Every ${n} minutes`;
  }

  if (!NUM.test(min)) return cron;
  const m = Number(min);
  if (m > 59) return cron;

  if (hour === '*' && everyDay) {
    return m === 0 ? 'Every hour' : `Every hour at :${String(m).padStart(2, '0')}`;
  }
  const hourStep = hour.match(/^\*\/(\d+)$/);
  if (hourStep && everyDay) {
    const n = Number(hourStep[1]);
    const base = n === 1 ? 'Every hour' : `Every ${n} hours`;
    return m === 0 ? base : `${base} at :${String(m).padStart(2, '0')}`;
  }

  // One fixed time or a short list of hours ("9,17").
  const hours = hour.split(',');
  if (!hours.every(h => NUM.test(h) && Number(h) <= 23)) return cron;
  const at = joinWords(hours.map(h => formatClock(Number(h), m)));

  if (dom === '*') {
    const days = parseDayOfWeek(dow);
    if (!days) return cron;
    return `${daysPhrase(days)} at ${at}`;
  }
  if (NUM.test(dom) && (dow === '*' || dow === '?')) {
    const d = Number(dom);
    if (d < 1 || d > 31) return cron;
    return `Monthly on the ${ordinal(d)} at ${at}`;
  }
  return cron;
}

/** True when `cronToEnglish` found words for this expression. */
export function cronHasWords(cron: string): boolean {
  return cronToEnglish(cron) !== cron;
}
