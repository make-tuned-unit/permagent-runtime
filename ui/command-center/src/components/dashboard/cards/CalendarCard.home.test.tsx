/**
 * @vitest-environment jsdom
 *
 * The Calendar card on Home: minimised (today's date, what is left of today,
 * Next up, today's list) until it is expanded into focus, where it is the
 * full Day / Week / Month calendar. Same shell mock as CalendarCard.test.tsx:
 * Tauri `invoke` answers `fullAccess` and serves events by range.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';

const apiFetch = vi.fn();
vi.mock('../../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));

const NOW = new Date(2026, 9, 7, 10, 0, 0).getTime(); // Wed 7 Oct 2026, 10:00
const at = (day: number, h: number, m = 0) => new Date(2026, 9, day, h, m).getTime();

const events = [
  { id: 'standup', title: 'Standup', startMs: at(7, 10, 42), endMs: at(7, 11), allDay: false, calendar: 'Work', attendees: 5 },
  { id: 'lunch', title: 'Lunch', startMs: at(7, 12), endMs: at(7, 13), allDay: false, calendar: 'Home', attendees: 0 },
  { id: 'early', title: 'Early run', startMs: at(7, 7), endMs: at(7, 8), allDay: false, calendar: 'Home', attendees: 0 },
  { id: 'trip', title: 'Offsite', startMs: at(7, 0), endMs: at(8, 0), allDay: true, calendar: 'Work', attendees: 0 },
];

const serve = async (cmd: string, args?: { start: string; end: string }) => {
  if (cmd === 'calendar_authorization_status') return 'fullAccess';
  if (cmd === 'calendar_events_in_range' && args) {
    const s = Date.parse(args.start); const e = Date.parse(args.end);
    return events.filter(x => x.startMs < e && x.endMs > s);
  }
  return undefined;
};
const invoke = vi.fn(serve);
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...(a as [string, never])) }));

import { CalendarCard } from './CalendarCard';
import { _clearCalendarCache } from './calendar/useCalendarEvents';
import { CardPresentationProvider, type CardPresentation } from '../homeFocus';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
const q = (id: string) => host.querySelector(`[data-testid="${id}"]`) as HTMLElement | null;
const expand = vi.fn();
const collapse = vi.fn();

async function settle() {
  for (let i = 0; i < 5; i += 1) await act(async () => { await new Promise(r => setTimeout(r, 0)); });
}

async function renderOnHome(presentation: CardPresentation, initialView?: 'day' | 'week' | 'month') {
  await act(async () => {
    root.render(
      <CardPresentationProvider value={{ presentation, expand, collapse }}>
        <CalendarCard nowMs={NOW} initialView={initialView} />
      </CardPresentationProvider>,
    );
  });
  await settle();
}

beforeEach(() => {
  _clearCalendarCache();
  invoke.mockReset().mockImplementation(serve);
  apiFetch.mockReset().mockRejectedValue(new Error('offline'));
  expand.mockReset();
  collapse.mockReset();
  try { localStorage.clear(); } catch { /* ignore */ }
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe('Calendar on Home, minimised', () => {
  it('shows a left-aligned date summary, equal-width view launchers, Next up and the agenda', async () => {
    await renderOnHome('overview');
    expect(q('calendar-card')!.dataset.mode).toBe('compact');
    // The compact agenda is still Today; these buttons launch a full calendar
    // view and do not falsely claim that today's visible list is that view.
    expect(host.querySelector('[role="tablist"]')).toBeNull();

    const header = q('calendar-compact-header')!;
    expect(header.tagName).toBe('DIV');
    expect(header.getAttribute('aria-expanded')).toBeNull();
    const longDate = new Date(NOW).toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
    expect(q('calendar-compact-date')!.textContent).toContain(longDate);
    expect(q('calendar-date-summary')!.style.alignItems).toBe('flex-start');
    const launchers = header.querySelector<HTMLElement>('[data-calendar-view-launcher]')!;
    expect(launchers.getAttribute('role')).toBe('group');
    expect(launchers.style.display).toBe('grid');
    expect(launchers.style.gridTemplateColumns).toBe('repeat(3, minmax(0, 1fr))');
    expect(header.style.flexWrap).toBe('wrap');
    expect(q('calendar-content-scroll')!.style.overflowY).toBe('auto');
    expect(q('calendar-content-scroll')!.contains(header)).toBe(false);
    expect(['day', 'week', 'month'].map(view => q(`calendar-open-${view}`)!.getAttribute('aria-label')))
      .toEqual(['Open Day calendar', 'Open Week calendar', 'Open Month calendar']);
    // Standup, Lunch and the all-day Offsite are still ahead; the 7:00 run is over.
    expect(q('calendar-today-summary')!.textContent).toBe('3 events left today');

    expect(q('calendar-next-up')!.textContent).toContain('in 42 min');
    const list = q('calendar-compact-list')!.textContent ?? '';
    expect(list).toContain('Standup');
    expect(list).toContain('Lunch');
    expect(list).not.toContain('Early run');
  });

  it('choosing a compact view opens that full calendar view without a backend write', async () => {
    await renderOnHome('overview', 'day');
    const callsBefore = invoke.mock.calls.length;
    await act(async () => q('calendar-open-week')!.click());
    expect(expand).toHaveBeenCalledOnce();
    expect(q('calendar-card')!.dataset.view).toBe('week');
    expect(localStorage.getItem('permagent.dashboard.calendar.view.v1')).toBe('week');
    expect(invoke.mock.calls.length).toBe(callsBefore);
    expect(invoke.mock.calls.some(([command]) => command === 'request_calendar_access')).toBe(false);

    await renderOnHome('focused');
    expect(q('calendar-card')!.dataset.mode).toBe('full');
    expect(q('calendar-week')).not.toBeNull();
    expect(q('calendar-view-week')!.getAttribute('aria-selected')).toBe('true');
    const panelId = q('calendar-view-week')!.getAttribute('aria-controls')!;
    expect(document.getElementById(panelId)).toBe(q('calendar-body'));
    expect(q('calendar-body')!.getAttribute('role')).toBe('tabpanel');
    expect(q('calendar-body')!.getAttribute('aria-labelledby')).toBe(q('calendar-view-week')!.id);
  });

  it.each(['day', 'week', 'month'] as const)('compact %s launcher opens the matching selected calendar view', async view => {
    await renderOnHome('overview');
    await act(async () => q(`calendar-open-${view}`)!.click());
    expect(expand).toHaveBeenCalledOnce();
    expect(q('calendar-card')!.dataset.view).toBe(view);
    await renderOnHome('focused');
    expect(q(`calendar-${view === 'day' ? 'dayview' : view}`)).not.toBeNull();
    expect(q(`calendar-view-${view}`)!.getAttribute('aria-selected')).toBe('true');
    expect([...host.querySelectorAll('[role="tab"]')].filter(tab => tab.getAttribute('aria-selected') === 'true'))
      .toEqual([q(`calendar-view-${view}`)]);
    expect(apiFetch.mock.calls.every(([, init]) => !['POST', 'PUT', 'PATCH', 'DELETE'].includes(init?.method ?? 'GET'))).toBe(true);
  });

  it('moves from a compact launcher to the full roving tab with arrow keys', async () => {
    await renderOnHome('overview');
    const day = q('calendar-open-day')!;
    day.focus();
    await act(async () => {
      day.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }));
    });
    expect(q('calendar-card')!.dataset.view).toBe('week');
    expect(expand).toHaveBeenCalledOnce();
    await renderOnHome('focused');
    expect(document.activeElement).toBe(q('calendar-view-week'));
  });

  it('keeps its own actions while minimised: an event still opens its details', async () => {
    await renderOnHome('overview');
    const row = Array.from(host.querySelectorAll<HTMLElement>('[data-testid="calendar-list-row"]'))
      .find(r => r.textContent?.includes('Lunch'))!;
    act(() => row.click());
    expect(document.querySelector('[data-testid="calendar-popover"]')?.getAttribute('aria-label')).toBe('Lunch');
    expect(expand).not.toHaveBeenCalled();
  });

  it('stays minimised as a compact tile while another card is focused', async () => {
    await renderOnHome('compact');
    expect(q('calendar-card')!.dataset.mode).toBe('compact');
    expect(q('calendar-compact-header')).not.toBeNull();
  });

  it('does not claim an empty day when no calendar is connected', async () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    await renderOnHome('overview');
    expect(q('calendar-today-summary')!.textContent).toBe('Calendar not connected');
    expect(q('calendar-access')!.dataset.state).toBe('desktop-only');
    expect(host.textContent).not.toContain('No events today');
    expect(q('calendar-next-up')).toBeNull();
    expect(q('calendar-compact-list')).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === 'request_calendar_access')).toBe(false);
  });

  it('reports a failed compact read honestly and retries the visible range', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'calendar_authorization_status') return 'fullAccess';
      throw new Error('Calendar read failed');
    });
    await renderOnHome('overview');
    expect(q('calendar-today-summary')!.textContent).toBe('Calendar unavailable');
    expect(q('calendar-next-up')!.textContent).toBe('Upcoming events could not be read');
    expect(q('calendar-failed')).not.toBeNull();
    expect(host.textContent).not.toContain('No events today');
    expect(host.textContent).not.toContain('Calendar not connected');
    invoke.mockImplementation(serve);
    act(() => q('calendar-retry')!.click());
    await settle();
    expect(q('calendar-failed')).toBeNull();
    expect(q('calendar-compact-list')!.textContent).toContain('Standup');
    expect(q('calendar-today-summary')!.textContent).toBe('3 events left today');
  });

  it('still distinguishes a verified empty day from an unread calendar', async () => {
    invoke.mockImplementation(async (command: string) => command === 'calendar_authorization_status' ? 'fullAccess' : []);
    await renderOnHome('overview');
    expect(q('calendar-today-summary')!.textContent).toBe('Nothing left today');
    expect(q('calendar-next-up')).toBeNull();
    expect(q('calendar-compact-list')).toBeNull();
    expect(host.textContent).not.toContain('No events today');
    expect(host.textContent).not.toContain('Nothing coming up in the next 7 days');
    expect(q('calendar-failed')).toBeNull();
  });
});

describe('Calendar on Home, focused', () => {
  it('does not claim an empty Day when no calendar source has been read', async () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    await renderOnHome('focused');
    act(() => q('calendar-view-day')!.click());
    await settle();
    expect(q('calendar-body')!.textContent).toContain('Connect a calendar to see events');
    expect(q('calendar-body')!.textContent).not.toContain('No events');
  });

  it('is the full calendar with its Day / Week / Month views', async () => {
    await renderOnHome('focused');
    expect(q('calendar-card')!.dataset.mode).toBe('full');
    expect(q('calendar-compact-header')).toBeNull();
    const tabs = Array.from(host.querySelectorAll('[role="tab"]')).map(t => t.textContent);
    expect(tabs).toEqual(['Day', 'Week', 'Month']);
    expect(q('calendar-week')).not.toBeNull();
    act(() => q('calendar-view-month')!.click());
    await settle();
    expect(q('calendar-month')).not.toBeNull();
  });
});
