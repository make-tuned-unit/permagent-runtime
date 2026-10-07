/**
 * @vitest-environment jsdom
 *
 * useCalendarEvents: which source answers, in what order, and what is cached.
 * Tauri `invoke` and `apiFetch` are mocked at the module boundary.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const apiFetch = vi.fn();
vi.mock('../../../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { _clearCalendarCache, REFRESH_MS, useCalendarEvents, type CalendarEvents } from './useCalendarEvents';
import { rangeFor } from './calendarModel';
import type { CalendarAuthStatus } from '../calendarAccess';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = new Date(2026, 9, 7, 10, 0, 0).getTime();
const WEEK = rangeFor('week', NOW, 0);
const LAST_WEEK = rangeFor('week', NOW - 7 * 86_400_000, 0);

let host: HTMLDivElement;
let root: Root;
let latest: CalendarEvents;

function Probe({ range, inDesktopApp, status }: {
  range: typeof WEEK; inDesktopApp: boolean; status: CalendarAuthStatus | null;
}) {
  latest = useCalendarEvents(range, { inDesktopApp, status }, NOW);
  return null;
}

async function mount(props: { range: typeof WEEK; inDesktopApp: boolean; status: CalendarAuthStatus | null }) {
  await act(async () => { root.render(<Probe {...props} />); });
  await act(async () => { await Promise.resolve(); });
}

const shellEvent = (title: string, startMs: number) => ({
  id: title, title, startMs, endMs: startMs + 3_600_000, allDay: false, calendar: 'Work', attendees: 2, url: null,
});

beforeEach(() => {
  _clearCalendarCache();
  apiFetch.mockReset();
  invoke.mockReset();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); });

describe('useCalendarEvents', () => {
  it('asks EventKit for the range as ISO timestamps once access is full', async () => {
    invoke.mockResolvedValue([shellEvent('Standup', NOW + 3_600_000)]);
    await mount({ range: WEEK, inDesktopApp: true, status: 'fullAccess' });
    expect(invoke).toHaveBeenCalledWith('calendar_events_in_range', {
      start: new Date(WEEK.startMs).toISOString(), end: new Date(WEEK.endMs).toISOString(),
    });
    expect(apiFetch).not.toHaveBeenCalled();
    expect(latest.source).toBe('eventkit');
    expect(latest.events.map(e => e.title)).toEqual(['Standup']);
    expect(latest.events[0].attendees).toBe(2);
  });

  it('falls back to the daemon when the shell rejects the range command', async () => {
    invoke.mockRejectedValue(new Error('command not found'));
    apiFetch.mockResolvedValue({ cells: [{ label: 'Call', value: 'Ada', sub: '2:30 PM' }] });
    await mount({ range: WEEK, inDesktopApp: true, status: 'fullAccess' });
    expect(apiFetch).toHaveBeenCalledWith('/api/dashboard/calendar');
    expect(latest.source).toBe('daemon');
    expect(latest.events[0]).toMatchObject({ title: 'Call', calendar: 'Ada' });
  });

  it('uses the daemon when access is not full or there is no desktop shell', async () => {
    apiFetch.mockResolvedValue({ cells: [{ label: 'Call', value: '', sub: '9:00 AM' }] });
    await mount({ range: WEEK, inDesktopApp: true, status: 'denied' });
    expect(invoke).not.toHaveBeenCalled();
    expect(latest.source).toBe('daemon');
    _clearCalendarCache();
    await mount({ range: { ...WEEK, startMs: WEEK.startMs + 1 }, inDesktopApp: false, status: null });
    expect(invoke).not.toHaveBeenCalled();
    expect(latest.source).toBe('daemon');
  });

  it('the daemon only answers a range that includes today', async () => {
    apiFetch.mockResolvedValue({ cells: [{ label: 'Call', value: '', sub: '9:00 AM' }] });
    await mount({ range: LAST_WEEK, inDesktopApp: false, status: null });
    expect(apiFetch).not.toHaveBeenCalled();
    expect(latest.events).toEqual([]);
  });

  it('reports none and failed when no source answers', async () => {
    apiFetch.mockRejectedValue(new Error('offline'));
    await mount({ range: WEEK, inDesktopApp: false, status: null });
    expect(latest.source).toBe('none');
    expect(latest.failed).toBe(true);
    expect(latest.events).toEqual([]);
  });

  it('waits for the access status instead of guessing a source', async () => {
    await mount({ range: WEEK, inDesktopApp: true, status: null });
    expect(invoke).not.toHaveBeenCalled();
    expect(apiFetch).not.toHaveBeenCalled();
    expect(latest.loading).toBe(true);
  });

  it('caches per range: paging back shows the earlier events at once, then revalidates', async () => {
    invoke.mockResolvedValue([shellEvent('A', NOW)]);
    await mount({ range: WEEK, inDesktopApp: true, status: 'fullAccess' });
    invoke.mockResolvedValue([shellEvent('B', LAST_WEEK.startMs + 1000)]);
    await mount({ range: LAST_WEEK, inDesktopApp: true, status: 'fullAccess' });
    expect(latest.events.map(e => e.title)).toEqual(['B']);

    // Back to the first range: the cache answers before the fetch resolves.
    let release!: (v: unknown) => void;
    invoke.mockReturnValue(new Promise(r => { release = r; }));
    await act(async () => { root.render(<Probe range={WEEK} inDesktopApp status="fullAccess" />); });
    expect(latest.events.map(e => e.title)).toEqual(['A']);
    expect(latest.loading).toBe(true);
    await act(async () => { release([shellEvent('A2', NOW)]); await Promise.resolve(); });
    expect(latest.events.map(e => e.title)).toEqual(['A2']);
  });

  it('refreshes every five minutes and when the window regains focus', async () => {
    vi.useFakeTimers();
    invoke.mockResolvedValue([]);
    await mount({ range: WEEK, inDesktopApp: true, status: 'fullAccess' });
    expect(invoke).toHaveBeenCalledTimes(1);
    await act(async () => { vi.advanceTimersByTime(REFRESH_MS); await Promise.resolve(); });
    expect(invoke).toHaveBeenCalledTimes(2);
    await act(async () => { window.dispatchEvent(new Event('focus')); await Promise.resolve(); });
    expect(invoke).toHaveBeenCalledTimes(3);
  });
});
