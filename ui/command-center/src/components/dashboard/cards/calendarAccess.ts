/**
 * Apple Calendar on the Home Calendar card — the pure half.
 *
 * The user's report: "i cant find anywhere to grant access to my iCal so my
 * calendar can sync". There was nowhere. The daemon reads Calendar with
 * AppleScript, macOS refuses it with -1743 and never prompts (launchd, not
 * Permagent.app, is the process it holds responsible), and nothing in the app
 * could ask. The desktop shell now asks EventKit itself — see
 * `ui/desktop/src-tauri/src/calendar.rs` — and this module turns what the shell
 * says into what the card shows.
 *
 * Kept free of React and Tauri so every branch is unit-testable: the status →
 * UI-state mapping, the event → card-cell mapping, and the merge with the
 * meetings the daemon still supplies from People.
 */

import type { CardCell } from './ManifestCard';

/**
 * EventKit authorization as `calendar_authorization_status` spells it.
 * `unsupported` covers "not macOS" and "the shell could not ask".
 */
export type CalendarAuthStatus =
  | 'notDetermined'
  | 'restricted'
  | 'denied'
  | 'fullAccess'
  | 'writeOnly'
  | 'unsupported';

const KNOWN_STATUSES: readonly CalendarAuthStatus[] = [
  'notDetermined', 'restricted', 'denied', 'fullAccess', 'writeOnly', 'unsupported',
];

/** Narrow whatever the shell returned; anything unexpected is `unsupported`. */
export function parseCalendarAuthStatus(raw: unknown): CalendarAuthStatus {
  return KNOWN_STATUSES.includes(raw as CalendarAuthStatus)
    ? (raw as CalendarAuthStatus)
    : 'unsupported';
}

/** One event from `calendar_upcoming_events` (Rust `CalendarEvent`). */
export interface ShellCalendarEvent {
  title: string;
  /** Unix epoch milliseconds. */
  startMs: number;
  endMs: number;
  allDay: boolean;
  /** Title of the calendar it belongs to ("Work", "Home"…). */
  calendar: string;
  location?: string | null;
}

/** The one action a state offers, if any. */
export type CalendarAccessAction = 'connect' | 'open-settings';

export interface CalendarAccessView {
  kind:
    | 'desktop-only'
    | 'checking'
    | 'connect'
    | 'denied'
    | 'restricted'
    | 'write-only'
    | 'unsupported'
    | 'connected';
  /** First line. Empty for `checking` and `connected`, which draw no panel. */
  text: string;
  hint?: string;
  action?: CalendarAccessAction;
  /** Button label for `action`. */
  actionLabel?: string;
}

/** Said in every state where Apple events are absent, so nobody wonders. */
const MEETINGS_STILL_SHOW = 'Meetings you log on a person still show up on this card.';

/**
 * What the card should say about Apple Calendar.
 *
 * @param inDesktopApp whether the Tauri shell is present at all. EventKit
 *   lives in the signed app; a browser tab has no way to ask for it, and must
 *   say so rather than show a button that cannot work.
 * @param status the shell's answer, or `null` while it is still being asked.
 * @param fullAccessAsked whether this card has already asked macOS for full
 *   access and got `writeOnly` back. macOS shows the write-only → full upgrade
 *   prompt once; after that only System Settings can change it.
 */
export function calendarAccessView(
  inDesktopApp: boolean,
  status: CalendarAuthStatus | null,
  fullAccessAsked = false,
): CalendarAccessView {
  if (!inDesktopApp) {
    return {
      kind: 'desktop-only',
      text: 'Apple Calendar connects in the Permagent desktop app',
      hint: `Open Permagent on your Mac and choose Connect Calendar on this card. ${MEETINGS_STILL_SHOW}`,
    };
  }
  switch (status) {
    case null:
      return { kind: 'checking', text: '' };
    case 'fullAccess':
      return { kind: 'connected', text: '' };
    case 'notDetermined':
      return {
        kind: 'connect',
        text: 'Show your Apple Calendar here',
        hint: `macOS will ask once whether Permagent may read your calendars. ${MEETINGS_STILL_SHOW}`,
        action: 'connect',
        actionLabel: 'Connect Calendar',
      };
    case 'denied':
      // macOS never prompts twice. After a "Don't Allow" the only way back is
      // the Privacy pane, so the button goes straight there.
      return {
        kind: 'denied',
        text: 'Calendar access is off — open System Settings',
        hint: `Turn on Permagent under Privacy & Security › Calendars, then come back here. ${MEETINGS_STILL_SHOW}`,
        action: 'open-settings',
        actionLabel: 'Open System Settings',
      };
    case 'writeOnly':
      // The installed app sat here with only "Open System Settings" on offer,
      // although macOS will still raise an "upgrade to full access" prompt for
      // an app that asks (TN3153). So the first control asks; only once macOS
      // has answered with write-only again does the card send the user to the
      // Privacy pane, which is then the one place left to change it.
      if (!fullAccessAsked) {
        return {
          kind: 'write-only',
          text: 'Permagent can add events but not read them',
          hint: `macOS will ask once whether Permagent may read your calendars too. ${MEETINGS_STILL_SHOW}`,
          action: 'connect',
          actionLabel: 'Allow Full Access',
        };
      }
      return {
        kind: 'write-only',
        text: 'Permagent can add events but not read them',
        hint: 'Switch Permagent to Full Access under Privacy & Security › Calendars to see your events here.',
        action: 'open-settings',
        actionLabel: 'Open System Settings',
      };
    case 'restricted':
      // Screen Time or a management profile. There is no switch the user can
      // flip for us, so no button pretends there is.
      return {
        kind: 'restricted',
        text: 'Calendar access is restricted on this Mac',
        hint: `A Screen Time or device-management setting blocks it, and Permagent cannot change that. ${MEETINGS_STILL_SHOW}`,
      };
    case 'unsupported':
    default:
      return {
        kind: 'unsupported',
        text: 'Apple Calendar is not available on this computer',
        hint: MEETINGS_STILL_SHOW,
      };
  }
}

/** Minutes past local midnight, for ordering. All-day events sort first. */
function minuteOfDay(ms: number): number {
  const d = new Date(ms);
  return d.getHours() * 60 + d.getMinutes();
}

function clockLabel(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

/** A card cell plus the sort key the merge orders by. */
interface KeyedCell {
  cell: CardCell;
  /** -1 = all day; null = unknown time (sorted last). */
  minute: number | null;
}

/** Event → list-layout cell: title, time (and place), calendar name. */
export function shellEventToCell(event: ShellCalendarEvent): CardCell {
  const when = event.allDay
    ? 'All day'
    : `${clockLabel(event.startMs)} – ${clockLabel(event.endMs)}`;
  const place = event.location?.trim();
  return {
    label: event.title.trim() || 'Untitled event',
    value: event.calendar,
    sub: place ? `${when} · ${place}` : when,
  };
}

/**
 * The daemon writes its times as `9:05 AM` (`fmt_clock` in
 * dashboard_cards.rs). Parse that back so People meetings interleave with the
 * Apple events by time instead of trailing after them.
 */
export function parseDaemonClock(sub: string | undefined): number | null {
  if (!sub) return null;
  const m = /^(\d{1,2}):(\d{2})\s*(AM|PM)$/i.exec(sub.trim());
  if (!m) return null;
  let hour = Number(m[1]) % 12;
  if (m[3].toUpperCase() === 'PM') hour += 12;
  return hour * 60 + Number(m[2]);
}

/**
 * Apple events from the shell, merged with the cells the daemon returned
 * (meetings and follow-ups logged on People, plus Apple events on the rare
 * setup where its AppleScript read still works).
 *
 * A daemon cell whose label matches an Apple event is the same meeting seen
 * twice — the daemon imports calendar events onto people by title — so it is
 * folded into the Apple event rather than listed again; its value (the
 * person's name) wins over the calendar name, because who you are meeting is
 * the more useful thing to read.
 */
export function mergeCalendarCells(
  events: readonly ShellCalendarEvent[],
  daemonCells: readonly CardCell[],
): CardCell[] {
  const keyed: KeyedCell[] = events.map(e => ({
    cell: shellEventToCell(e),
    minute: e.allDay ? -1 : minuteOfDay(e.startMs),
  }));

  for (const d of daemonCells) {
    const same = keyed.find(k => k.cell.label.toLowerCase() === d.label.trim().toLowerCase());
    if (same) {
      if (d.value !== '' && d.value != null) same.cell = { ...same.cell, value: d.value };
      continue;
    }
    keyed.push({ cell: d, minute: parseDaemonClock(d.sub) });
  }

  // Stable sort: equal keys keep Apple-first, then daemon order.
  return keyed
    .map((k, i) => ({ ...k, i }))
    .sort((a, b) => {
      const am = a.minute ?? Number.MAX_SAFE_INTEGER;
      const bm = b.minute ?? Number.MAX_SAFE_INTEGER;
      return am - bm || a.i - b.i;
    })
    .map(k => k.cell);
}
