import { describe, expect, it } from 'vitest';
import {
  addDays, countdownLabel, fromDaemonCells, fromShellEvent, layoutDay, monthDayChips, monthGrid,
  nextUp, nextUpText, rangeFor, shiftAnchor, startOfDay, startOfWeek, upcomingEvents, weekDays,
  type CalEvent,
} from './calendarModel';

// Wednesday 7 October 2026, 10:00 local.
const NOW = new Date(2026, 9, 7, 10, 0, 0).getTime();
const at = (day: number, h: number, m = 0) => new Date(2026, 9, day, h, m).getTime();

function ev(id: string, startMs: number, endMs: number, over: Partial<CalEvent> = {}): CalEvent {
  return {
    id, title: id, startMs, endMs, allDay: false, calendar: 'Work', attendees: 0, source: 'eventkit', ...over,
  };
}

describe('ranges and grids', () => {
  it('a month range is always 6 weeks starting on the first weekday', () => {
    for (const firstDay of [0, 1]) {
      const r = rangeFor('month', NOW, firstDay);
      expect(new Date(r.startMs).getDay()).toBe(firstDay);
      expect(Math.round((r.endMs - r.startMs) / 86_400_000)).toBe(42);
      // The grid contains the whole of October.
      expect(r.startMs).toBeLessThanOrEqual(at(1, 0));
      expect(r.endMs).toBeGreaterThan(at(31, 23));
    }
  });

  it('monthGrid has 42 cells, flags the month and today', () => {
    const cells = monthGrid(NOW, NOW, 0);
    expect(cells).toHaveLength(42);
    expect(cells.filter(c => c.inMonth)).toHaveLength(31);
    expect(cells.filter(c => c.isToday).map(c => new Date(c.dayMs).getDate())).toEqual([7]);
    expect(new Date(cells[0].dayMs).getDay()).toBe(0);
  });

  it('week and day ranges', () => {
    const w = rangeFor('week', NOW, 1);
    expect(new Date(w.startMs).getDay()).toBe(1);
    expect(weekDays(NOW, 1)).toHaveLength(7);
    expect(startOfWeek(NOW, 0)).toBe(at(4, 0));
    expect(rangeFor('day', NOW, 0)).toEqual({ startMs: at(7, 0), endMs: at(8, 0) });
  });

  it('shiftAnchor steps a month without overshooting short months', () => {
    const jan31 = new Date(2026, 0, 31).getTime();
    expect(new Date(shiftAnchor('month', jan31, 1)).getMonth()).toBe(1);
    expect(new Date(shiftAnchor('month', jan31, 1)).getDate()).toBe(28);
    expect(new Date(shiftAnchor('week', NOW, -1)).getDate()).toBe(30); // 30 Sep
    expect(new Date(shiftAnchor('day', NOW, 1)).getDate()).toBe(8);
  });
});

describe('month chips', () => {
  it('shows at most 3 chips and counts the rest', () => {
    const events = Array.from({ length: 5 }, (_, i) => ev(`e${i}`, at(7, 9 + i), at(7, 10 + i)));
    const { shown, hidden } = monthDayChips(events, at(7, 0), 3);
    expect(shown).toHaveLength(3);
    expect(hidden).toBe(2);
    expect(monthDayChips(events.slice(0, 3), at(7, 0), 3).hidden).toBe(0);
  });

  it('puts all-day first and includes a multi-day event on each day it covers', () => {
    const trip = ev('trip', at(6, 0), at(9, 0), { allDay: true });
    const talk = ev('talk', at(7, 9), at(7, 10));
    expect(monthDayChips([talk, trip], at(7, 0)).shown.map(e => e.id)).toEqual(['trip', 'talk']);
    expect(monthDayChips([trip], at(8, 0)).shown).toHaveLength(1);
    expect(monthDayChips([trip], at(9, 0)).shown).toHaveLength(0); // end is exclusive
  });
});

describe('week overlap layout', () => {
  it('lays overlapping events side by side and leaves lone events full width', () => {
    const a = ev('a', at(7, 9), at(7, 11));
    const b = ev('b', at(7, 10), at(7, 12));
    const c = ev('c', at(7, 10, 30), at(7, 11, 30));
    const lone = ev('lone', at(7, 14), at(7, 15));
    const placed = layoutDay([a, b, c, lone], at(7, 0));
    const by = Object.fromEntries(placed.map(p => [p.event.id, p]));
    expect(by.a.cols).toBe(3);
    expect(new Set([by.a.col, by.b.col, by.c.col]).size).toBe(3);
    expect(by.lone).toMatchObject({ col: 0, cols: 1, top: 14 * 60, height: 60 });
  });

  it('reuses a freed column inside a cluster', () => {
    const a = ev('a', at(7, 9), at(7, 10));
    const b = ev('b', at(7, 9), at(7, 12));
    const c = ev('c', at(7, 10), at(7, 11));
    const by = Object.fromEntries(layoutDay([a, b, c], at(7, 0)).map(p => [p.event.id, p]));
    expect(by.c.col).toBe(by.a.col);
    expect(by.a.cols).toBe(2);
  });

  it('clips an event that crosses midnight and drops all-day events from the grid', () => {
    const late = ev('late', at(7, 23), at(8, 1));
    const allDay = ev('ad', at(7, 0), at(8, 0), { allDay: true });
    const p = layoutDay([late, allDay], at(7, 0));
    expect(p).toHaveLength(1);
    expect(p[0].top + p[0].height).toBe(24 * 60);
    expect(layoutDay([late], at(8, 0))[0]).toMatchObject({ top: 0, height: 60 });
  });

  it('gives a zero-length event a visible minimum height', () => {
    expect(layoutDay([ev('z', at(7, 9), at(7, 9))], at(7, 0))[0].height).toBeGreaterThanOrEqual(20);
  });
});

describe('next up', () => {
  const standup = ev('Standup', at(7, 10, 42), at(7, 11), { calendar: 'Work' });
  it('counts down in whole minutes, rounding up', () => {
    const n = nextUp([standup], NOW)!;
    expect(n.state).toBe('upcoming');
    expect(n.minutes).toBe(42);
    expect(nextUpText(n)).toBe('Next: Standup · in 42 min · Work');
    expect(nextUp([standup], NOW + 30_000)!.minutes).toBe(42); // 41m30s -> 42
  });

  it('prefers the event in progress and says when it ends', () => {
    const running = ev('Review', at(7, 9, 30), at(7, 10, 20));
    const n = nextUp([standup, running], NOW)!;
    expect(n.state).toBe('ongoing');
    expect(n.minutes).toBe(20);
    expect(nextUpText(n)).toBe('Now: Review · ends in 20 min · Work');
  });

  it('skips finished and all-day events, and is null with nothing left', () => {
    const done = ev('done', at(7, 8), at(7, 9));
    const allDay = ev('holiday', at(7, 0), at(8, 0), { allDay: true });
    expect(nextUp([done, allDay], NOW)).toBeNull();
    expect(nextUp([], NOW)).toBeNull();
  });

  it('formats the countdown at minute precision', () => {
    expect(countdownLabel(0)).toBe('now');
    expect(countdownLabel(1)).toBe('in 1 min');
    expect(countdownLabel(59)).toBe('in 59 min');
    expect(countdownLabel(60)).toBe('in 1 h');
    expect(countdownLabel(125)).toBe('in 2 h 5 min');
    expect(countdownLabel(60 * 24 * 3)).toBe('in 3 days');
  });

  it('upcomingEvents returns the next n still to come', () => {
    const list = [ev('a', at(7, 8), at(7, 9)), ev('b', at(7, 11), at(7, 12)), ev('c', at(7, 13), at(7, 14)),
      ev('d', at(8, 9), at(8, 10)), ev('e', at(9, 9), at(9, 10))];
    expect(upcomingEvents(list, NOW, 3).map(e => e.id)).toEqual(['b', 'c', 'd']);
  });
});

describe('sources map to one shape', () => {
  it('fromShellEvent fills defaults', () => {
    const e = fromShellEvent({ startMs: 5, endMs: 3, title: '  ', calendar: null });
    expect(e).toMatchObject({ title: 'Untitled event', endMs: 5, attendees: 0, calendar: '', source: 'eventkit' });
  });

  it('fromDaemonCells places clock cells on today and untimed cells all day', () => {
    const [timed, untimed] = fromDaemonCells([
      { label: 'Call', value: 'Ada', sub: '2:30 PM' },
      { label: 'Birthday', value: '', sub: undefined },
    ], NOW);
    expect(timed).toMatchObject({ startMs: at(7, 14, 30), endMs: at(7, 15, 30), allDay: false, calendar: 'Ada', source: 'daemon' });
    expect(untimed).toMatchObject({ allDay: true, startMs: startOfDay(NOW), endMs: addDays(startOfDay(NOW), 1) });
  });
});
