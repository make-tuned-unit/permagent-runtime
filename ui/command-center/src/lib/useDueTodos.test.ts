import { describe, it, expect } from 'vitest';
import {
  localIsoDate,
  daysUntil,
  bucketFor,
  relativeDueLabel,
  groupByBucket,
  BUCKET_ORDER,
  type DueTodo,
} from './useDueTodos';

/**
 * The daemon decides WHICH to-dos are due; this module decides how each date
 * reads relative to today. Nearly every way that second job goes wrong is a
 * date bug that only shows up for some users at some hours — so the awkward
 * cases (timezones, DST, month ends, midnight) are pinned explicitly.
 */

function todo(id: string, dueDate: string, extra: Partial<DueTodo> = {}): DueTodo {
  return {
    id,
    title: `todo ${id}`,
    projectId: 'p1',
    projectName: 'Project One',
    columnId: 'c1',
    columnName: 'Backlog',
    dueDate,
    assignedTo: null,
    updatedAt: '2026-08-02T00:00:00Z',
    ...extra,
  };
}

describe('localIsoDate', () => {
  it('reports the LOCAL calendar day, not the UTC one', () => {
    // 2026-08-02 21:30 in a UTC+5 zone is already 2026-08-03 in UTC. Using
    // toISOString() here would file a to-do due today under "overdue" for
    // every user east of Greenwich in the evening.
    const evening = new Date(2026, 7, 2, 21, 30, 0);
    expect(localIsoDate(evening)).toBe('2026-08-02');
  });

  it('zero-pads single-digit months and days', () => {
    expect(localIsoDate(new Date(2026, 0, 5))).toBe('2026-01-05');
  });

  it('handles the last instant before midnight', () => {
    expect(localIsoDate(new Date(2026, 11, 31, 23, 59, 59))).toBe('2026-12-31');
  });
});

describe('daysUntil', () => {
  it('is zero for today', () => {
    expect(daysUntil('2026-08-02', '2026-08-02')).toBe(0);
  });

  it('counts forward and backward', () => {
    expect(daysUntil('2026-08-05', '2026-08-02')).toBe(3);
    expect(daysUntil('2026-07-30', '2026-08-02')).toBe(-3);
  });

  it('crosses month and year boundaries', () => {
    expect(daysUntil('2026-09-01', '2026-08-31')).toBe(1);
    expect(daysUntil('2027-01-01', '2026-12-31')).toBe(1);
  });

  it('counts a leap day', () => {
    expect(daysUntil('2028-03-01', '2028-02-28')).toBe(2);
  });

  it('gives a whole day across a DST transition', () => {
    // Both sides parse as UTC midnight, so a spring-forward day is still 1 day
    // and cannot round down to 0.
    expect(daysUntil('2026-03-09', '2026-03-08')).toBe(1);
    expect(daysUntil('2026-11-02', '2026-11-01')).toBe(1);
  });

  it('degrades to 0 on an unparseable date rather than throwing', () => {
    expect(daysUntil('not-a-date', '2026-08-02')).toBe(0);
  });
});

describe('bucketFor', () => {
  const today = '2026-08-02';
  it('files a past date as overdue', () => {
    expect(bucketFor('2026-08-01', today)).toBe('overdue');
    expect(bucketFor('2025-01-01', today)).toBe('overdue');
  });
  it('files today and tomorrow separately', () => {
    expect(bucketFor('2026-08-02', today)).toBe('today');
    expect(bucketFor('2026-08-03', today)).toBe('tomorrow');
  });
  it('files 2..7 days out as this week, 8+ as later', () => {
    expect(bucketFor('2026-08-04', today)).toBe('week');
    expect(bucketFor('2026-08-09', today)).toBe('week');
    expect(bucketFor('2026-08-10', today)).toBe('later');
  });
});

describe('relativeDueLabel', () => {
  const today = '2026-08-02';
  it('reads naturally at each boundary', () => {
    expect(relativeDueLabel('2026-08-02', today)).toBe('due today');
    expect(relativeDueLabel('2026-08-03', today)).toBe('due tomorrow');
    expect(relativeDueLabel('2026-08-01', today)).toBe('1 day overdue');
    expect(relativeDueLabel('2026-07-28', today)).toBe('5 days overdue');
    expect(relativeDueLabel('2026-08-09', today)).toBe('in 7 days');
  });

  it('singularises one day overdue', () => {
    // "1 days overdue" is the kind of thing that makes a UI look unfinished.
    expect(relativeDueLabel('2026-08-01', today)).not.toContain('1 days');
  });
});

describe('groupByBucket', () => {
  const today = '2026-08-02';

  it('returns groups in overdue-first display order', () => {
    const groups = groupByBucket([
      todo('later', '2026-09-01'),
      todo('today', '2026-08-02'),
      todo('overdue', '2026-07-01'),
      todo('tomorrow', '2026-08-03'),
      todo('week', '2026-08-06'),
    ], today);
    expect(groups.map(g => g.bucket)).toEqual(BUCKET_ORDER);
    expect(groups[0].label).toBe('Overdue');
  });

  it('omits empty buckets entirely', () => {
    const groups = groupByBucket([todo('a', '2026-08-02')], today);
    expect(groups).toHaveLength(1);
    expect(groups[0].bucket).toBe('today');
  });

  it('preserves the order the daemon sent within a bucket', () => {
    // The daemon sorts by date ascending; grouping must not reshuffle.
    const groups = groupByBucket([
      todo('first', '2026-07-01'),
      todo('second', '2026-07-15'),
      todo('third', '2026-08-01'),
    ], today);
    expect(groups[0].todos.map(t => t.id)).toEqual(['first', 'second', 'third']);
  });

  it('returns nothing for an empty list', () => {
    expect(groupByBucket([], today)).toEqual([]);
  });

  it('keeps every to-do — none is dropped by bucketing', () => {
    const input = [
      todo('a', '2026-07-01'), todo('b', '2026-08-02'), todo('c', '2026-08-03'),
      todo('d', '2026-08-07'), todo('e', '2027-01-01'),
    ];
    const total = groupByBucket(input, today).reduce((n, g) => n + g.todos.length, 0);
    expect(total).toBe(input.length);
  });
});
