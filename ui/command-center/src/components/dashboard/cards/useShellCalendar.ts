/**
 * Apple Calendar through the desktop shell — the live half.
 *
 * Asks `calendar_authorization_status` on mount, reads today's events with
 * `calendar_upcoming_events` once access is granted, and exposes the two
 * actions the Calendar card offers: `connect` (the one-time macOS prompt, or
 * the one-time upgrade prompt from write-only access) and
 * `openSettings` (the Privacy › Calendars pane, the only way back after a
 * "Don't Allow"). See `src-tauri/src/calendar.rs` for why this lives in the
 * app shell and not the daemon.
 *
 * The status is re-read whenever the window regains focus: the user who was
 * sent to System Settings comes back to a card that already knows they flipped
 * the switch, instead of one still telling them to go and flip it.
 *
 * Outside Tauri (a browser tab, a test) nothing is invoked and `inDesktopApp`
 * is false, which the card turns into "connects in the desktop app".
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import {
  parseCalendarAuthStatus,
  parseCalendarRequestResult,
  type CalendarAuthStatus,
  type ShellCalendarEvent,
} from './calendarAccess';

/** Today only — the card is "Today's events". */
const DAYS = 1;
const DEFAULT_REFRESH_SECONDS = 300;

function detectDesktopApp(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

async function invokeShell<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

export interface ShellCalendar {
  inDesktopApp: boolean;
  /** `null` while the first status read is in flight. */
  status: CalendarAuthStatus | null;
  /** Today's Apple events once readable; empty otherwise. */
  events: ShellCalendarEvent[];
  /**
   * True once a `connect` in this session was ANSWERED with `writeOnly`:
   * macOS has shown (or already spent) its one upgrade prompt, so the card
   * offers System Settings instead of asking again. A request that timed out
   * unanswered leaves this false — nobody saw the prompt.
   */
  fullAccessAsked: boolean;
  /** Ask macOS for access. Resolves `true` only when access was granted. */
  connect: () => Promise<boolean>;
  /** Open System Settings › Privacy & Security › Calendars. */
  openSettings: () => Promise<boolean>;
}

/**
 * @param enabled only the Calendar card turns this on; every other manifest
 *   card calls the hook (rules of hooks) and gets an inert result.
 */
export function useShellCalendar(
  enabled: boolean,
  refreshSeconds?: number,
  /**
   * Read today's events with every status refresh. The range-based Calendar
   * card turns this off: it asks `calendar_events_in_range` for its own window
   * and only needs this hook for the access state and the two actions.
   */
  loadToday = true,
): ShellCalendar {
  const [inDesktopApp] = useState(detectDesktopApp);
  const [status, setStatus] = useState<CalendarAuthStatus | null>(null);
  const [events, setEvents] = useState<ShellCalendarEvent[]>([]);
  const [fullAccessAsked, setFullAccessAsked] = useState(false);
  const alive = useRef(true);
  // The request waits for the user to answer the system prompt (up to five
  // minutes in calendar.rs). A second click meanwhile would only queue a
  // second wait, so it is ignored.
  const requesting = useRef(false);

  const active = enabled && inDesktopApp;

  const loadEvents = useCallback(async () => {
    try {
      const list = await invokeShell<ShellCalendarEvent[]>('calendar_upcoming_events', { days: DAYS });
      if (alive.current) setEvents(Array.isArray(list) ? list : []);
    } catch {
      // Keep the last good list: a failed refresh should not blank the card.
    }
  }, []);

  const refresh = useCallback(async () => {
    let next: CalendarAuthStatus;
    try {
      next = parseCalendarAuthStatus(await invokeShell<string>('calendar_authorization_status'));
    } catch {
      // The status command is infallible in calendar.rs, so a rejection means
      // this app does not register it ("command not found"): an outdated
      // shell, not a Mac without Calendar. Saying "not available on this
      // computer" here sent the user looking for a problem an update fixes.
      next = 'shellOutdated';
    }
    if (!alive.current) return next;
    setStatus(next);
    if (next === 'fullAccess') {
      if (loadToday) await loadEvents();
    } else {
      // Access revoked in System Settings while the app was open: the events
      // we already hold are no longer ours to show.
      setEvents([]);
    }
    return next;
  }, [loadEvents, loadToday]);

  useEffect(() => {
    alive.current = true;
    if (!active) return () => { alive.current = false; };

    void refresh();
    const seconds = refreshSeconds && refreshSeconds > 0 ? refreshSeconds : DEFAULT_REFRESH_SECONDS;
    const timer = setInterval(() => { void refresh(); }, seconds * 1000);
    const onFocus = () => { void refresh(); };
    window.addEventListener('focus', onFocus);
    return () => {
      alive.current = false;
      clearInterval(timer);
      window.removeEventListener('focus', onFocus);
    };
  }, [active, refresh, refreshSeconds]);

  const connect = useCallback(async () => {
    if (!active || requesting.current) return false;
    requesting.current = true;
    try {
      const result = parseCalendarRequestResult(await invokeShell<string>('request_calendar_access'));
      if (!result.answered) {
        // The prompt went unanswered (a locked screen, as during QA, for the
        // whole wait). Re-read the status so a late answer still shows, but keep
        // offering the ask: treating this like a write-only answer sent the
        // user to System Settings for a prompt they never saw.
        if (!alive.current) return false;
        return (await refresh()) === 'fullAccess';
      }
      const next = result.status ?? 'unsupported';
      if (!alive.current) return next === 'fullAccess';
      setStatus(next);
      if (next === 'writeOnly') setFullAccessAsked(true);
      if (next === 'fullAccess' && loadToday) await loadEvents();
      return next === 'fullAccess';
    } catch {
      return false;
    } finally {
      requesting.current = false;
    }
  }, [active, loadEvents, loadToday, refresh]);

  const openSettings = useCallback(async () => {
    if (!active) return false;
    try {
      await invokeShell<void>('open_calendar_privacy_settings');
      return true;
    } catch {
      return false;
    }
  }, [active]);

  return { inDesktopApp, status, events, fullAccessAsked, connect, openSettings };
}
