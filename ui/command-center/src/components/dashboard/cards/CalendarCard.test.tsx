/**
 * @vitest-environment jsdom
 *
 * The range-based Calendar card: views, period navigation, keyboard, Next up,
 * the compact size and the event popover. Tauri `invoke` is mocked; the shell
 * answers `fullAccess` and serves events by range.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const apiFetch = vi.fn();
vi.mock('../../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));

const NOW = new Date(2026, 9, 7, 10, 0, 0).getTime(); // Wed 7 Oct 2026, 10:00
const at = (day: number, h: number, m = 0) => new Date(2026, 9, day, h, m).getTime();

const events = [
  { id: 'standup', title: 'Standup', startMs: at(7, 10, 42), endMs: at(7, 11), allDay: false, calendar: 'Work', location: 'Room 4', attendees: 5, url: 'https://meet.example.test/x' },
  { id: 'lunch', title: 'Lunch', startMs: at(7, 12), endMs: at(7, 13), allDay: false, calendar: 'Home', attendees: 0 },
  { id: 'trip', title: 'Offsite', startMs: at(7, 0), endMs: at(8, 0), allDay: true, calendar: 'Work', attendees: 0 },
  ...[0, 1, 2, 3, 4].map(i => ({ id: `m${i}`, title: `Busy ${i}`, startMs: at(14, 9 + i), endMs: at(14, 10 + i), allDay: false, calendar: 'Work', attendees: 0 })),
];

const invoke = vi.fn(async (cmd: string, args?: { start: string; end: string }) => {
  if (cmd === 'calendar_authorization_status') return 'fullAccess';
  if (cmd === 'calendar_events_in_range' && args) {
    const s = Date.parse(args.start); const e = Date.parse(args.end);
    return events.filter(x => x.startMs < e && x.endMs > s);
  }
  return undefined;
});
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...(a as [string, never])) }));

import { CalendarCard, VIEW_STORAGE_KEY } from './CalendarCard';
import { _clearCalendarCache } from './calendar/useCalendarEvents';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
const q = (id: string) => host.querySelector(`[data-testid="${id}"]`) as HTMLElement | null;
const qa = (id: string) => Array.from(host.querySelectorAll(`[data-testid="${id}"]`)) as HTMLElement[];

async function render(props: Parameters<typeof CalendarCard>[0] = {}) {
  await act(async () => { root.render(<CalendarCard nowMs={NOW} sizeMode="full" {...props} />); });
  // The access status resolves first (a dynamic import, then `invoke`), and
  // only then are the events asked for: let the whole chain settle.
  for (let i = 0; i < 4; i += 1) await act(async () => { await new Promise(r => setTimeout(r, 0)); });
}
async function key(k: string, target: Element = q('calendar-card')!) {
  await act(async () => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
    await Promise.resolve();
  });
  for (let i = 0; i < 3; i += 1) await act(async () => { await new Promise(r => setTimeout(r, 0)); });
}

beforeEach(() => {
  _clearCalendarCache();
  invoke.mockClear();
  apiFetch.mockReset().mockRejectedValue(new Error('offline'));
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

describe('Next up', () => {
  it('shows the next event with a minute countdown and its calendar', async () => {
    await render();
    const strip = q('calendar-next-up')!;
    expect(strip.dataset.state).toBe('upcoming');
    expect(strip.textContent).toContain('Standup');
    expect(strip.textContent).toContain('in 42 min');
    expect(strip.textContent).toContain('Work');
  });

  it('counts down as the clock moves', async () => {
    await render({ nowMs: at(7, 10, 12) });
    expect(q('calendar-countdown')!.textContent).toContain('in 30 min');
  });
});

describe('views and keyboard', () => {
  it('defaults to Week and remembers the view', async () => {
    await render();
    expect(q('calendar-card')!.dataset.view).toBe('week');
    expect(q('calendar-week')).not.toBeNull();
    await key('m');
    expect(q('calendar-month')).not.toBeNull();
    expect(localStorage.getItem(VIEW_STORAGE_KEY)).toBe('month');
    act(() => root.unmount());
    root = createRoot(host);
    await render();
    expect(q('calendar-card')!.dataset.view).toBe('month');
  });

  it('D / W / M switch views and the tabs agree', async () => {
    await render();
    await key('d');
    expect(q('calendar-dayview')).not.toBeNull();
    expect(q('calendar-view-day')!.getAttribute('aria-selected')).toBe('true');
    await key('w');
    expect(q('calendar-week')).not.toBeNull();
  });

  it('arrow keys move the period and T returns to today', async () => {
    await render({ initialView: 'month' });
    const title = () => q('calendar-title')!.textContent;
    const start = title();
    expect(start).toMatch(/October/);
    await key('ArrowRight');
    expect(title()).toMatch(/November/);
    await key('ArrowLeft');
    await key('ArrowLeft');
    expect(title()).toMatch(/September/);
    await key('t');
    expect(title()).toBe(start);
  });

  it('ignores shortcuts with a modifier held', async () => {
    await render();
    await act(async () => {
      q('calendar-card')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'm', metaKey: true, bubbles: true }));
    });
    expect(q('calendar-week')).not.toBeNull();
  });

  it('asks the shell for the visible range when paging', async () => {
    await render({ initialView: 'week' });
    invoke.mockClear();
    await key('ArrowRight');
    const call = invoke.mock.calls.find(c => c[0] === 'calendar_events_in_range');
    expect(call).toBeTruthy();
    expect(Date.parse((call![1] as unknown as { start: string }).start)).toBeGreaterThan(at(10, 0) - 1);
  });
});

describe('month', () => {
  it('draws 42 cells, 3 chips and +N for a busy day, and +N opens that day', async () => {
    await render({ initialView: 'month' });
    expect(qa('calendar-month-cell')).toHaveLength(42);
    const busy = qa('calendar-month-cell').find(c => c.dataset.day === String(at(14, 0)))!;
    expect(busy.querySelectorAll('[data-testid="calendar-chip"]')).toHaveLength(3);
    const more = busy.querySelector('[data-testid="calendar-more"]') as HTMLElement;
    expect(more.textContent).toContain('+2 more');
    await act(async () => { more.click(); await Promise.resolve(); });
    expect(q('calendar-dayview')).not.toBeNull();
    expect(q('calendar-title')!.textContent).toMatch(/14/);
  });
});

describe('week and day', () => {
  it('week has seven columns, an all-day row and a now line on today', async () => {
    await render({ initialView: 'week' });
    expect(qa('calendar-day-col')).toHaveLength(7);
    expect(q('calendar-allday')!.textContent).toContain('Offsite');
    expect(qa('calendar-now-line')).toHaveLength(1);
    expect(qa('calendar-block').map(b => b.getAttribute('aria-label'))).toEqual(
      expect.arrayContaining([expect.stringContaining('Standup'), expect.stringContaining('Lunch')]),
    );
  });

  it('day lists the events beside the timeline', async () => {
    await render({ initialView: 'day' });
    expect(qa('calendar-list-row').map(r => r.textContent)).toEqual(
      expect.arrayContaining([expect.stringContaining('Standup'), expect.stringContaining('Offsite')]),
    );
  });
});

describe('popover', () => {
  it('opens on an event with when, where, calendar, attendees; Escape closes', async () => {
    await render({ initialView: 'day' });
    const row = qa('calendar-list-row').find(r => r.textContent!.includes('Standup'))!;
    await act(async () => { row.click(); await Promise.resolve(); });
    const pop = document.querySelector('[data-testid="calendar-popover"]') as HTMLElement;
    expect(pop).not.toBeNull();
    expect(pop.textContent).toContain('Room 4');
    expect(pop.textContent).toContain('Work');
    expect(pop.textContent).toContain('5 people');
    expect(pop.textContent).toContain('Open link');
    expect(pop.textContent).toContain('Open in Calendar');
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(document.querySelector('[data-testid="calendar-popover"]')).toBeNull();
  });
});

describe('compact size', () => {
  it('shows only Next up and today, with no header controls', async () => {
    await render({ sizeMode: 'compact' });
    expect(q('calendar-card')!.dataset.mode).toBe('compact');
    expect(q('calendar-next-up')!.textContent).toContain('Standup');
    expect(q('calendar-compact-list')!.textContent).toContain('Lunch');
    expect(q('calendar-view-month')).toBeNull();
    expect(q('calendar-week')).toBeNull();
  });

  it('lists the next three when nothing is left today', async () => {
    await render({ sizeMode: 'compact', nowMs: at(7, 20) });
    const list = q('calendar-compact-list')!;
    expect(list.textContent).toContain('Coming up');
    expect(list.querySelectorAll('[data-testid="calendar-list-row"]').length).toBeLessThanOrEqual(3);
  });
});

describe('without a desktop shell', () => {
  it('says Apple Calendar connects in the desktop app when no source answers', async () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    await render();
    expect(q('calendar-access')!.dataset.state).toBe('desktop-only');
  });
});
