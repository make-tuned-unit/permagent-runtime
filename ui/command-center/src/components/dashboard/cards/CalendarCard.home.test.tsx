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

async function renderOnHome(presentation: CardPresentation) {
  await act(async () => {
    root.render(
      <CardPresentationProvider value={{ presentation, expand, collapse }}>
        <CalendarCard nowMs={NOW} />
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
  it('shows today at a glance: the date, what is left of today, Next up and the list', async () => {
    await renderOnHome('overview');
    expect(q('calendar-card')!.dataset.mode).toBe('compact');
    // No Day/Week/Month chrome while minimised.
    expect(host.querySelector('[role="tablist"]')).toBeNull();

    const header = q('calendar-compact-header')!;
    const longDate = new Date(NOW).toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
    expect(header.textContent).toContain(longDate);
    // Standup, Lunch and the all-day Offsite are still ahead; the 7:00 run is over.
    expect(q('calendar-today-summary')!.textContent).toBe('3 events left today');

    expect(q('calendar-next-up')!.textContent).toContain('in 42 min');
    const list = q('calendar-compact-list')!.textContent ?? '';
    expect(list).toContain('Standup');
    expect(list).toContain('Lunch');
    expect(list).not.toContain('Early run');
  });

  it('expands on click, and says what expanding opens', async () => {
    await renderOnHome('overview');
    const header = q('calendar-compact-header')!;
    expect(header.tagName).toBe('BUTTON');
    expect(header.getAttribute('aria-expanded')).toBe('false');
    expect(header.getAttribute('aria-label')).toMatch(/^Expand calendar — .*3 events left today\. Opens Day, Week and Month views\.$/);
    act(() => header.click());
    expect(expand).toHaveBeenCalledTimes(1);
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
  });
});

describe('Calendar on Home, focused', () => {
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
