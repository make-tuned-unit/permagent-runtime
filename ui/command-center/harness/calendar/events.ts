// 40 mocked events across two weeks around Wed 7 Oct 2026.
const at = (day: number, h: number, m = 0) => new Date(2026, 9, day, h, m).getTime();
let n = 0;
const E = (title: string, day: number, h: number, m: number, mins: number, calendar: string, extra: Record<string, unknown> = {}) => ({
  id: `e${n++}`, title, startMs: at(day, h, m), endMs: at(day, h, m) + mins * 60_000, allDay: false,
  calendar, attendees: 0, ...extra,
});
const D = (title: string, day: number, days: number, calendar: string) => ({
  id: `e${n++}`, title, startMs: at(day, 0), endMs: at(day + days, 0), allDay: true, calendar, attendees: 0,
});

export const MOCK_EVENTS = [
  E('Standup', 5, 9, 30, 15, 'Work', { attendees: 6 }), E('Planning', 5, 14, 0, 60, 'Work', { location: 'Room 4', attendees: 8 }),
  E('Standup', 6, 9, 30, 15, 'Work', { attendees: 6 }), E('Dentist', 6, 16, 30, 45, 'Personal', { location: '12 Mill St' }),
  E('Standup', 7, 9, 30, 15, 'Work', { attendees: 6 }), E('Design review: calendar card and a very long title that must truncate', 7, 10, 42, 60, 'Work', { location: 'Studio', attendees: 4, url: 'https://meet.example.test/abc' }),
  E('1:1 with Sam', 7, 11, 0, 30, 'Work', { attendees: 2 }), E('Customer call', 7, 11, 15, 45, 'Work', { attendees: 3 }),
  E('Lunch with Priya', 7, 12, 30, 60, 'Personal', { location: 'Cafe Nord' }), E('Focus block', 7, 15, 0, 120, 'Focus'),
  E('Pick up parcel', 7, 18, 0, 20, 'Personal'),
  E('Standup', 8, 9, 30, 15, 'Work', { attendees: 6 }), E('Retro', 8, 15, 0, 60, 'Work', { attendees: 7 }),
  E('Yoga', 8, 18, 30, 60, 'Personal'),
  E('Standup', 9, 9, 30, 15, 'Work', { attendees: 6 }), E('Investor update', 9, 13, 0, 90, 'Work', { attendees: 5 }),
  E('Dinner', 9, 19, 30, 120, 'Personal', { location: 'Home' }),
  E('Board prep', 10, 10, 0, 90, 'Work'), E('Gym', 10, 7, 0, 60, 'Personal'),
  D('Offsite', 7, 1, 'Work'), D('Sam on leave', 12, 3, 'Work'), D('Sprint 42', 5, 5, 'Focus'),
  E('Standup', 12, 9, 30, 15, 'Work', { attendees: 6 }), E('Roadmap', 12, 11, 0, 60, 'Work', { attendees: 9 }),
  E('Standup', 13, 9, 30, 15, 'Work', { attendees: 6 }), E('Interview', 13, 14, 0, 45, 'Work', { attendees: 3 }),
  E('Interview', 13, 14, 30, 45, 'Work', { attendees: 3 }), E('Debrief', 13, 15, 30, 30, 'Work'),
  E('Standup', 14, 9, 30, 15, 'Work', { attendees: 6 }), E('Q4 kickoff', 14, 10, 0, 60, 'Work'),
  E('Review A', 14, 11, 30, 30, 'Work'), E('Review B', 14, 12, 0, 30, 'Work'), E('Review C', 14, 13, 0, 30, 'Work'),
  E('Review D', 14, 14, 0, 30, 'Work'), E('Pottery', 14, 18, 0, 90, 'Personal'),
  E('Standup', 15, 9, 30, 15, 'Work', { attendees: 6 }), E('Demo day', 15, 16, 0, 60, 'Work', { attendees: 20 }),
  E('Train to Leeds', 16, 8, 0, 120, 'Personal'),
  E('Standup', 19, 9, 30, 15, 'Work'),
];
