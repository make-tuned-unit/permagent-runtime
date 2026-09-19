/**
 * The pure half of the Calendar card's Apple Calendar access: EventKit status
 * → what the card says and which control it offers, and shell events merged
 * with the People meetings the daemon still supplies.
 *
 * The bug behind it: "i cant find anywhere to grant access to my iCal so my
 * calendar can sync". These pin that every status either carries a control
 * that works or says why there is none, and that moving the Apple read into
 * the shell does not drop meetings logged on a person.
 */
import { describe, expect, it } from 'vitest';
import {
  calendarAccessView,
  mergeCalendarCells,
  parseCalendarAuthStatus,
  parseDaemonClock,
  type ShellCalendarEvent,
} from './calendarAccess';

describe('calendarAccessView', () => {
  it('outside the desktop app says it needs the desktop app, with no button', () => {
    for (const status of [null, 'notDetermined', 'fullAccess', 'denied'] as const) {
      const v = calendarAccessView(false, status);
      expect(v.kind).toBe('desktop-only');
      expect(v.text).toContain('desktop app');
      expect(v.action).toBeUndefined();
    }
  });

  it('notDetermined offers Connect Calendar', () => {
    const v = calendarAccessView(true, 'notDetermined');
    expect(v).toMatchObject({ kind: 'connect', action: 'connect', actionLabel: 'Connect Calendar' });
  });

  it('denied points at System Settings, because macOS never prompts twice', () => {
    const v = calendarAccessView(true, 'denied');
    expect(v).toMatchObject({ kind: 'denied', action: 'open-settings' });
    expect(v.text).toBe('Calendar access is off — open System Settings');
  });

  it('writeOnly first asks macOS for full access, since it still shows the upgrade prompt once', () => {
    // The installed app sat in write-only with only "Open System Settings".
    expect(calendarAccessView(true, 'writeOnly')).toMatchObject({
      kind: 'write-only', action: 'connect', actionLabel: 'Allow Full Access',
    });
  });

  it('writeOnly routes to System Settings once macOS has answered the upgrade with write-only', () => {
    expect(calendarAccessView(true, 'writeOnly', true)).toMatchObject({
      kind: 'write-only', action: 'open-settings', actionLabel: 'Open System Settings',
    });
  });

  it('restricted offers no button: there is no switch the user can flip', () => {
    const v = calendarAccessView(true, 'restricted');
    expect(v.kind).toBe('restricted');
    expect(v.action).toBeUndefined();
  });

  it('fullAccess and the first read draw no panel', () => {
    expect(calendarAccessView(true, 'fullAccess')).toEqual({ kind: 'connected', text: '' });
    expect(calendarAccessView(true, null)).toEqual({ kind: 'checking', text: '' });
  });

  it('never tells the user to "grant" anything without a control to do it', () => {
    const statuses = ['notDetermined', 'restricted', 'denied', 'fullAccess', 'writeOnly', 'unsupported'] as const;
    for (const s of statuses) {
      const v = calendarAccessView(true, s);
      if (/grant/i.test(`${v.text} ${v.hint ?? ''}`)) expect(v.action).toBeDefined();
    }
  });
});

describe('parseCalendarAuthStatus', () => {
  it('passes known names and maps anything else to unsupported', () => {
    expect(parseCalendarAuthStatus('fullAccess')).toBe('fullAccess');
    expect(parseCalendarAuthStatus('authorized')).toBe('unsupported');
    expect(parseCalendarAuthStatus(undefined)).toBe('unsupported');
  });
});

describe('parseDaemonClock', () => {
  it('reads the daemon fmt_clock format', () => {
    expect(parseDaemonClock('9:05 AM')).toBe(9 * 60 + 5);
    expect(parseDaemonClock('12:00 AM')).toBe(0);
    expect(parseDaemonClock('12:30 PM')).toBe(12 * 60 + 30);
    expect(parseDaemonClock('2:00 PM')).toBe(14 * 60);
    expect(parseDaemonClock(undefined)).toBeNull();
    expect(parseDaemonClock('soon')).toBeNull();
  });
});

function at(h: number, m = 0): number {
  const d = new Date();
  d.setHours(h, m, 0, 0);
  return d.getTime();
}

const standup: ShellCalendarEvent = {
  title: 'Standup', startMs: at(9), endMs: at(9, 15), allDay: false, calendar: 'Work', location: null,
};
const lunch: ShellCalendarEvent = {
  title: 'Lunch with Sam', startMs: at(12, 30), endMs: at(13, 30), allDay: false, calendar: 'Home', location: 'Cafe',
};
const holiday: ShellCalendarEvent = {
  title: 'Bank holiday', startMs: at(0), endMs: at(23, 59), allDay: true, calendar: 'Holidays',
};

describe('mergeCalendarCells', () => {
  it('keeps People meetings when Apple events come from the shell, ordered by time', () => {
    const cells = mergeCalendarCells([lunch, standup, holiday], [
      { label: 'Intro call', value: 'Alex Kim', sub: '10:30 AM' },
      { label: 'Follow up with Priya', value: 'Send deck', sub: '4:00 PM' },
    ]);
    expect(cells.map(c => c.label)).toEqual([
      'Bank holiday', 'Standup', 'Intro call', 'Lunch with Sam', 'Follow up with Priya',
    ]);
    expect(cells[0].sub).toBe('All day');
    expect(cells[3].sub).toContain('Cafe');
    expect(cells[3].value).toBe('Home');
  });

  it('folds a People meeting that is the same Apple event, keeping the person', () => {
    const cells = mergeCalendarCells([lunch], [{ label: 'lunch with sam', value: 'Sam Lee', sub: '12:30 PM' }]);
    expect(cells).toHaveLength(1);
    expect(cells[0].value).toBe('Sam Lee');
  });

  it('with no shell events returns the daemon cells, sorted by their clock', () => {
    const daemon = [{ label: 'A', value: '', sub: '3:00 PM' }, { label: 'B', value: '', sub: '8:00 AM' }];
    expect(mergeCalendarCells([], daemon).map(c => c.label)).toEqual(['B', 'A']);
  });
});
