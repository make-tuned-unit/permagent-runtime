/**
 * The Calendar card's pure half: dates, ranges, grids, overlap layout, the
 * "Next up" arithmetic and the calendar-colour mapping. No React, no Tauri, so
 * every branch is unit-testable and the views stay a thin drawing layer.
 *
 * All times are local. Events carry Unix epoch milliseconds (the shell's and
 * the daemon fallback's common currency); "a day" is local midnight to the next
 * local midnight, which is 23 or 25 hours on a DST change, so day arithmetic
 * goes through `Date#setDate` and never adds 86 400 000.
 */

import type { ThemeColors } from '../../../../styles/tokens';
import { parseDaemonClock } from '../calendarAccess';
import type { CardCell } from '../ManifestCard';

export type CalendarView = 'day' | 'week' | 'month';
export const CALENDAR_VIEWS: readonly CalendarView[] = ['day', 'week', 'month'];

export const MINUTE_MS = 60_000;
export const HOUR_MS = 60 * MINUTE_MS;

/** One event as every view draws it, whichever source supplied it. */
export interface CalEvent {
  /** Stable per source; recurring occurrences share it, so key on `eventKey`. */
  id: string;
  title: string;
  startMs: number;
  endMs: number;
  allDay: boolean;
  /** Title of the calendar it lives in ("Work", "Home"...). */
  calendar: string;
  location?: string;
  attendees: number;
  url?: string;
  source: 'eventkit' | 'daemon';
}

export function eventKey(e: CalEvent): string {
  return `${e.id}|${e.startMs}`;
}

/** What `calendar_events_in_range` sends (Rust `CalendarEvent`, camelCase). */
export interface RawShellEvent {
  id?: string | null;
  title?: string | null;
  startMs: number;
  endMs: number;
  allDay?: boolean;
  calendar?: string | null;
  location?: string | null;
  attendees?: number | null;
  url?: string | null;
}

export function fromShellEvent(raw: RawShellEvent): CalEvent {
  const title = (raw.title ?? '').trim() || 'Untitled event';
  return {
    id: raw.id || `${title}|${raw.startMs}`,
    title,
    startMs: raw.startMs,
    endMs: Math.max(raw.endMs, raw.startMs),
    allDay: !!raw.allDay,
    calendar: (raw.calendar ?? '').trim(),
    location: raw.location?.trim() || undefined,
    attendees: Math.max(0, Math.floor(raw.attendees ?? 0)),
    url: raw.url?.trim() || undefined,
    source: 'eventkit',
  };
}

/**
 * The daemon's `/api/dashboard/calendar` answers with today's cells only and a
 * 12-hour clock string, no end time and no date. They become today's events: a
 * cell with a readable clock is a one-hour event, one without is all-day.
 */
export function fromDaemonCells(cells: readonly CardCell[], todayMs: number): CalEvent[] {
  const day = startOfDay(todayMs);
  return cells.map((c, i) => {
    const minute = parseDaemonClock(c.sub);
    const start = minute == null ? day : day + minute * MINUTE_MS;
    const person = typeof c.value === 'string' ? c.value.trim() : '';
    return {
      id: `daemon:${i}:${c.label}`,
      title: c.label.trim() || 'Untitled event',
      startMs: start,
      endMs: minute == null ? addDays(day, 1) : start + HOUR_MS,
      allDay: minute == null,
      calendar: person || 'People',
      attendees: 0,
      source: 'daemon' as const,
    };
  });
}

// ── dates ─────────────────────────────────────────────────────────────────

export function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

export function addDays(ms: number, n: number): number {
  const d = new Date(ms);
  d.setDate(d.getDate() + n);
  return d.getTime();
}

export function sameDay(a: number, b: number): boolean {
  return startOfDay(a) === startOfDay(b);
}

/** 0 = Sunday ... 6 = Saturday. The user's locale decides when it can say. */
export function localeFirstDay(): number {
  try {
    const loc = new Intl.Locale(navigator.language) as Intl.Locale & {
      weekInfo?: { firstDay: number };
      getWeekInfo?: () => { firstDay: number };
    };
    const info = loc.getWeekInfo?.() ?? loc.weekInfo;
    if (info && typeof info.firstDay === 'number') return info.firstDay % 7; // ISO 7 = Sunday
  } catch { /* fall through */ }
  return 0;
}

export function startOfWeek(ms: number, firstDay: number): number {
  const sod = startOfDay(ms);
  const back = (new Date(sod).getDay() - firstDay + 7) % 7;
  return addDays(sod, -back);
}

export interface DateRange { startMs: number; endMs: number }

/** The window a view needs: the day, the week, or the 6-week month grid. */
export function rangeFor(view: CalendarView, anchorMs: number, firstDay: number): DateRange {
  if (view === 'day') {
    const s = startOfDay(anchorMs);
    return { startMs: s, endMs: addDays(s, 1) };
  }
  if (view === 'week') {
    const s = startOfWeek(anchorMs, firstDay);
    return { startMs: s, endMs: addDays(s, 7) };
  }
  const first = new Date(anchorMs);
  first.setDate(1);
  const s = startOfWeek(first.getTime(), firstDay);
  return { startMs: s, endMs: addDays(s, 42) };
}

/** Step the anchor by one period of the view. */
export function shiftAnchor(view: CalendarView, anchorMs: number, dir: -1 | 1): number {
  if (view === 'day') return addDays(anchorMs, dir);
  if (view === 'week') return addDays(anchorMs, 7 * dir);
  const d = new Date(anchorMs);
  const day = d.getDate();
  d.setDate(1);
  d.setMonth(d.getMonth() + dir);
  // Jan 31 -> Feb: clamp to the new month's last day, not March 3rd.
  const last = new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
  d.setDate(Math.min(day, last));
  return d.getTime();
}

export interface MonthCell { dayMs: number; inMonth: boolean; isToday: boolean }

/** Always 42 cells (6 weeks), so the grid never changes height between months. */
export function monthGrid(anchorMs: number, nowMs: number, firstDay: number): MonthCell[] {
  const { startMs } = rangeFor('month', anchorMs, firstDay);
  const month = new Date(anchorMs).getMonth();
  const today = startOfDay(nowMs);
  return Array.from({ length: 42 }, (_, i) => {
    const dayMs = addDays(startMs, i);
    return { dayMs, inMonth: new Date(dayMs).getMonth() === month, isToday: dayMs === today };
  });
}

export function weekDays(anchorMs: number, firstDay: number): number[] {
  const s = startOfWeek(anchorMs, firstDay);
  return Array.from({ length: 7 }, (_, i) => addDays(s, i));
}

export function periodTitle(view: CalendarView, anchorMs: number, firstDay: number): string {
  const loc = undefined; // the user's own locale
  if (view === 'month') {
    return new Date(anchorMs).toLocaleDateString(loc, { month: 'long', year: 'numeric' });
  }
  if (view === 'day') {
    return new Date(anchorMs).toLocaleDateString(loc, { weekday: 'short', month: 'short', day: 'numeric' });
  }
  const days = weekDays(anchorMs, firstDay);
  const a = new Date(days[0]);
  const b = new Date(days[6]);
  const sameMonth = a.getMonth() === b.getMonth();
  const left = a.toLocaleDateString(loc, { month: 'short', day: 'numeric' });
  const right = sameMonth
    ? String(b.getDate())
    : b.toLocaleDateString(loc, { month: 'short', day: 'numeric' });
  return `${left} – ${right}, ${b.getFullYear()}`;
}

export function clockLabel(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

export function whenLabel(e: CalEvent): string {
  const day = new Date(e.startMs).toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' });
  if (e.allDay) {
    const lastDay = addDays(startOfDay(e.endMs - 1), 0);
    if (lastDay > startOfDay(e.startMs)) {
      const end = new Date(lastDay).toLocaleDateString([], { month: 'short', day: 'numeric' });
      return `${day} – ${end} · All day`;
    }
    return `${day} · All day`;
  }
  return `${day} · ${clockLabel(e.startMs)} – ${clockLabel(e.endMs)}`;
}

// ── selecting events ──────────────────────────────────────────────────────

export function sortEvents(events: readonly CalEvent[]): CalEvent[] {
  return [...events].sort((a, b) =>
    Number(b.allDay) - Number(a.allDay) || a.startMs - b.startMs || b.endMs - a.endMs
    || a.title.localeCompare(b.title));
}

/** Whether `e` touches the local day starting at `dayMs`. */
export function overlapsDay(e: CalEvent, dayMs: number): boolean {
  const end = addDays(dayMs, 1);
  if (e.endMs <= e.startMs) return e.startMs >= dayMs && e.startMs < end; // instant
  return e.startMs < end && e.endMs > dayMs;
}

export function eventsOnDay(events: readonly CalEvent[], dayMs: number): CalEvent[] {
  return sortEvents(events.filter(e => overlapsDay(e, dayMs)));
}

export interface DayChips { shown: CalEvent[]; hidden: number }

/** A month cell's chips: at most `max`, with the rest folded into "+N". */
export function monthDayChips(events: readonly CalEvent[], dayMs: number, max = 3): DayChips {
  const all = eventsOnDay(events, dayMs);
  if (all.length <= max) return { shown: all, hidden: 0 };
  return { shown: all.slice(0, max), hidden: all.length - max };
}

// ── time-grid layout (Day and Week) ───────────────────────────────────────

export interface PlacedEvent {
  event: CalEvent;
  /** Minutes from the day's local midnight, clipped to the day. */
  top: number;
  height: number;
  /** Column index inside its overlap cluster, and the cluster's width. */
  col: number;
  cols: number;
}

const MIN_BLOCK_MINUTES = 30;

/**
 * Timed events for one day, laid side by side where they overlap.
 *
 * Events are clustered (a chain of overlaps shares one cluster), then each is
 * dropped into the first column of its cluster that is free at its start. A
 * cluster's `cols` is the widest it ever got, so a lone event inside a busy
 * morning is not squeezed by a clash it never touches.
 */
export function layoutDay(events: readonly CalEvent[], dayMs: number): PlacedEvent[] {
  const dayEnd = addDays(dayMs, 1);
  const timed = events
    .filter(e => !e.allDay && overlapsDay(e, dayMs))
    .map(e => {
      const s = Math.max(e.startMs, dayMs);
      const en = Math.min(Math.max(e.endMs, e.startMs), dayEnd);
      const top = (s - dayMs) / MINUTE_MS;
      const height = Math.max((en - s) / MINUTE_MS, MIN_BLOCK_MINUTES);
      return { event: e, top, height };
    })
    .sort((a, b) => a.top - b.top || b.height - a.height);

  const out: PlacedEvent[] = [];
  let cluster: { event: CalEvent; top: number; height: number; col: number }[] = [];
  let clusterEnd = -Infinity;
  const colEnds: number[] = [];

  const flush = () => {
    const cols = Math.max(1, colEnds.length);
    for (const c of cluster) out.push({ ...c, cols });
    cluster = [];
    colEnds.length = 0;
    clusterEnd = -Infinity;
  };

  for (const item of timed) {
    if (cluster.length > 0 && item.top >= clusterEnd) flush();
    let col = colEnds.findIndex(end => end <= item.top);
    if (col === -1) { col = colEnds.length; colEnds.push(0); }
    colEnds[col] = item.top + item.height;
    clusterEnd = Math.max(clusterEnd, item.top + item.height);
    cluster.push({ ...item, col });
  }
  flush();
  return out;
}

// ── Next up ───────────────────────────────────────────────────────────────

export interface NextUp {
  event: CalEvent;
  state: 'ongoing' | 'upcoming';
  /** Whole minutes until it starts (upcoming) or ends (ongoing); never < 0. */
  minutes: number;
}

/** The event that matters next: one in progress, else the first to start. */
export function nextUp(events: readonly CalEvent[], nowMs: number): NextUp | null {
  const live = events
    .filter(e => !e.allDay && e.endMs > nowMs)
    .sort((a, b) => a.startMs - b.startMs || a.endMs - b.endMs);
  const e = live[0];
  if (!e) return null;
  if (e.startMs <= nowMs) {
    return { event: e, state: 'ongoing', minutes: Math.max(0, Math.ceil((e.endMs - nowMs) / MINUTE_MS)) };
  }
  return { event: e, state: 'upcoming', minutes: Math.max(0, Math.ceil((e.startMs - nowMs) / MINUTE_MS)) };
}

/** The next `n` events still to come (or running), soonest first. */
export function upcomingEvents(events: readonly CalEvent[], nowMs: number, n: number): CalEvent[] {
  return events
    .filter(e => e.endMs > nowMs && !(e.allDay && e.startMs > addDays(startOfDay(nowMs), 1)))
    .sort((a, b) => a.startMs - b.startMs || Number(b.allDay) - Number(a.allDay))
    .slice(0, n);
}

/** "in 42 min", "in 2 h 5 min", "in 3 days"; minute precision. */
export function countdownLabel(minutes: number): string {
  if (minutes < 1) return 'now';
  if (minutes < 60) return `in ${minutes} min`;
  if (minutes < 24 * 60) {
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    return m ? `in ${h} h ${m} min` : `in ${h} h`;
  }
  const d = Math.round(minutes / (24 * 60));
  return `in ${d} ${d === 1 ? 'day' : 'days'}`;
}

export function nextUpText(n: NextUp): string {
  const when = n.state === 'ongoing'
    ? (n.minutes < 1 ? 'ending now' : `ends ${countdownLabel(n.minutes)}`)
    : countdownLabel(n.minutes);
  return `${n.state === 'ongoing' ? 'Now' : 'Next'}: ${n.event.title} · ${when}${n.event.calendar ? ` · ${n.event.calendar}` : ''}`;
}

// ── colour ────────────────────────────────────────────────────────────────

/** The theme hues a calendar can take, in the order a hash picks from them. */
export function calendarPalette(colors: ThemeColors): string[] {
  // No red: it is the now-line's colour and the app's alarm colour, and a
  // calendar called "Work" should not read as a fault.
  return [colors.cyan, colors.purpleBright, colors.success, colors.warning, colors.stale];
}

/**
 * A calendar's colour, from the theme. EventKit has the user's own colour per
 * calendar, but a raw hex from outside the theme would not survive both
 * themes' contrast rules, so the calendar NAME picks a stable hue from the
 * palette instead: the same calendar is the same colour everywhere, in light
 * and dark, and two calendars rarely collide.
 */
export function calendarColor(name: string, colors: ThemeColors): string {
  let h = 0;
  for (let i = 0; i < name.length; i += 1) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  const palette = calendarPalette(colors);
  return palette[h % palette.length];
}

/** A soft wash of a hue, for chip and block fills. */
export function wash(hue: string, percent = 18, over = 'transparent'): string {
  return `color-mix(in srgb, ${hue} ${percent}%, ${over})`;
}

/**
 * Horizontal placement of a block in its overlap cluster, in percent of the
 * day column. Two clashing events sit side by side; three or more would be
 * slivers in a column a seventh of the card wide, so each keeps at least
 * `MIN_CLUSTER_WIDTH` and later ones overlap the earlier ones' right edge
 * (drawn on top, as every desktop calendar does) — columns still run left to
 * right in the order the model assigned them.
 */
export const MIN_CLUSTER_WIDTH = 56;
export function clusterBox(col: number, cols: number): { left: number; width: number } {
  if (cols <= 1) return { left: 0, width: 100 };
  const width = cols === 2 ? 50 : Math.max(100 / cols, MIN_CLUSTER_WIDTH);
  const left = col * ((100 - width) / (cols - 1));
  return { left, width };
}
