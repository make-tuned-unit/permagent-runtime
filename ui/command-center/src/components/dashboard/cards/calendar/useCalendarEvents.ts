/**
 * Events for an arbitrary date range, from the best source there is.
 *
 *   1. The desktop shell's EventKit bridge (`calendar_events_in_range`) once the
 *      Calendar access status is `fullAccess`.
 *   2. The daemon route (`/api/dashboard/calendar`) otherwise. It only knows
 *      TODAY (AppleScript read plus meetings logged on People), so it answers
 *      for a range only as far as the range includes today.
 *   3. Nothing: the card says honestly that Apple Calendar connects in the
 *      desktop app.
 *
 * Results are cached per source and range at module level, so paging back to a
 * month already seen is instant and a re-mounted card does not start empty;
 * every read still revalidates, on mount, on window focus and every five
 * minutes. Nothing here blocks render: the first paint is whatever the cache
 * holds (possibly nothing) and the fetch lands afterwards.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { apiFetch } from '../../../../lib/api';
import type { CardData } from '../ManifestCard';
import type { ShellCalendar } from '../useShellCalendar';
import {
  addDays, fromDaemonCells, fromShellEvent, startOfDay,
  type CalEvent, type DateRange, type RawShellEvent,
} from './calendarModel';

export type CalendarSource = 'eventkit' | 'daemon' | 'none';

export const REFRESH_MS = 5 * 60_000;

interface CacheEntry { at: number; events: CalEvent[]; source: CalendarSource }
const cache = new Map<string, CacheEntry>();

/** Tests start from an empty cache. */
export function _clearCalendarCache(): void { cache.clear(); }

const keyOf = (source: CalendarSource, r: DateRange) => `${source}|${r.startMs}|${r.endMs}`;

async function invokeShell<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

export async function fetchShellRange(range: DateRange): Promise<CalEvent[]> {
  const list = await invokeShell<RawShellEvent[]>('calendar_events_in_range', {
    start: new Date(range.startMs).toISOString(),
    end: new Date(range.endMs).toISOString(),
  });
  return (Array.isArray(list) ? list : []).map(fromShellEvent);
}

export async function fetchDaemonRange(range: DateRange, nowMs: number): Promise<CalEvent[]> {
  const today = startOfDay(nowMs);
  // The route has no date parameter: outside today there is nothing to ask.
  if (today >= range.endMs || addDays(today, 1) <= range.startMs) return [];
  const data = await apiFetch<CardData>('/api/dashboard/calendar');
  return fromDaemonCells(data?.cells ?? [], nowMs)
    .filter(e => e.startMs < range.endMs && e.endMs > range.startMs);
}

export interface CalendarEvents {
  events: CalEvent[];
  source: CalendarSource;
  /** True while a fetch is in flight (cached events may already be showing). */
  loading: boolean;
  /** The last fetch failed and nothing cached covers the range. */
  failed: boolean;
  refresh: () => void;
}

/**
 * @param range the window to read; `null` pauses the hook.
 * @param shell the already-running `useShellCalendar` (access state + actions).
 * @param nowMs only the daemon source uses it, to place its time-only cells on today.
 */
export function useCalendarEvents(
  range: DateRange | null,
  shell: Pick<ShellCalendar, 'inDesktopApp' | 'status'>,
  nowMs: number = Date.now(),
): CalendarEvents {
  const checking = shell.inDesktopApp && shell.status === null;
  const preferred: CalendarSource = shell.inDesktopApp && shell.status === 'fullAccess' ? 'eventkit' : 'daemon';

  const startMs = range?.startMs ?? 0;
  const endMs = range?.endMs ?? 0;
  const [state, setState] = useState<{ key: string; events: CalEvent[]; source: CalendarSource; failed: boolean }>(
    () => {
      const hit = range ? cache.get(keyOf(preferred, range)) : undefined;
      return { key: range ? keyOf(preferred, range) : '', events: hit?.events ?? [], source: hit?.source ?? 'none', failed: false };
    },
  );
  const [loading, setLoading] = useState(false);
  const generation = useRef(0);
  const nowRef = useRef(nowMs);
  nowRef.current = nowMs;

  const load = useCallback(async () => {
    if (!range || checking) return;
    const gen = ++generation.current;
    const r = { startMs, endMs };
    setLoading(true);
    let source: CalendarSource = preferred;
    let events: CalEvent[] | null = null;
    if (preferred === 'eventkit') {
      try {
        events = await fetchShellRange(r);
      } catch {
        // An outdated shell has no range command, and a revoked grant rejects
        // it: neither is a reason to blank the card, the daemon is next.
        events = null;
        source = 'daemon';
      }
    }
    if (events === null) {
      try {
        events = await fetchDaemonRange(r, nowRef.current);
        source = 'daemon';
      } catch {
        events = null;
      }
    }
    if (gen !== generation.current) return;
    setLoading(false);
    if (events === null) {
      const kept = cache.get(keyOf(preferred, r));
      setState({ key: keyOf(preferred, r), events: kept?.events ?? [], source: kept?.source ?? 'none', failed: !kept });
      return;
    }
    cache.set(keyOf(preferred, r), { at: Date.now(), events, source });
    setState({ key: keyOf(preferred, r), events, source, failed: false });
  // `range` is represented by its two numbers.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [startMs, endMs, preferred, checking, range === null]);

  // A new range or source shows the cache at once (stale-while-revalidate).
  useEffect(() => {
    if (!range) return;
    const hit = cache.get(keyOf(preferred, { startMs, endMs }));
    setState(prev => (hit
      ? { key: keyOf(preferred, { startMs, endMs }), events: hit.events, source: hit.source, failed: false }
      : prev));
    void load();
    const timer = setInterval(() => { void load(); }, REFRESH_MS);
    const onFocus = () => { void load(); };
    window.addEventListener('focus', onFocus);
    return () => {
      generation.current += 1;
      clearInterval(timer);
      window.removeEventListener('focus', onFocus);
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load]);

  // Events of a different window must never be drawn into this one.
  const current = range && state.key === keyOf(preferred, { startMs, endMs });
  const events = current ? state.events : [];
  return {
    events,
    source: current ? state.source : 'none',
    loading: loading || checking,
    failed: !!range && !checking && !loading && !!current && state.failed,
    refresh: () => { void load(); },
  };
}
