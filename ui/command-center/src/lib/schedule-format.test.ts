/**
 * QA 2026-09-24: the Automate cards read "0 0 6 * * 1-5" as "Day 6 of every
 * month at 12 AM". croner is SIX fields (sec min hour dom mon dow); the helper
 * read the first five as min hour dom mon dow. The roster showed the raw
 * string. Both now read this one helper.
 */
import { describe, expect, it } from 'vitest';
import { cronHasWords, cronToEnglish, parseDayOfWeek } from './schedule-format';

describe('cronToEnglish — the live expressions from the QA walk (6-field croner)', () => {
  // Each of these used to read as "Day N of every month at …": the seconds
  // field was taken for the minute, so the hour landed in day-of-month.
  it.each([
    ['0 0 6 * * 1-5', 'Weekdays at 6:00 AM'],
    ['0 0 8 * * 1-5', 'Weekdays at 8:00 AM'],
    ['0 20 3 * * *', 'Daily at 3:20 AM'],
    ['0 0 10 * * 1,4', 'Mon & Thu at 10:00 AM'],
    ['0 30 19 * * 0', 'Sundays at 7:30 PM'],
    ['0 0 19 * * 0', 'Sundays at 7:00 PM'],
  ])('%s → %s', (cron, words) => {
    expect(cronToEnglish(cron)).toBe(words);
  });
});

describe('cronToEnglish — other shapes (6-field croner)', () => {
  it.each([
    ['0 0 6 * * 1-5', 'Weekdays at 6:00 AM'],
    ['0 0 10 * * 1,4', 'Mon & Thu at 10:00 AM'],
    ['0 30 19 * * 0', 'Sundays at 7:30 PM'],
    ['0 30 19 * * 7', 'Sundays at 7:30 PM'],
    ['0 0 8 * * *', 'Daily at 8:00 AM'],
    ['0 0 8 * * * *', 'Daily at 8:00 AM'],
    ['0 0 0 * * *', 'Daily at 12:00 AM'],
    ['0 0 12 * * 0,6', 'Weekends at 12:00 PM'],
    ['0 15 9 * * 1,3,5', 'Mon, Wed & Fri at 9:15 AM'],
    ['0 0 9 * * MON-FRI', 'Weekdays at 9:00 AM'],
    ['0 0 9,17 * * 1-5', 'Weekdays at 9:00 AM & 5:00 PM'],
    ['0 0 7 * * 5-7', 'Fri, Sat & Sun at 7:00 AM'],
    ['0 0 9 1 * *', 'Monthly on the 1st at 9:00 AM'],
    ['0 */15 * * * *', 'Every 15 minutes'],
    ['0 0 * * * *', 'Every hour'],
    ['0 0 */4 * * *', 'Every 4 hours'],
  ])('%s → %s', (cron, words) => {
    expect(cronToEnglish(cron)).toBe(words);
  });
});

describe('cronToEnglish — 5-field input still reads', () => {
  it.each([
    ['0 8 * * 1-5', 'Weekdays at 8:00 AM'],
    ['0 19 * * 0', 'Sundays at 7:00 PM'],
    ['0 0 1 * *', 'Monthly on the 1st at 12:00 AM'],
  ])('%s → %s', (cron, words) => {
    expect(cronToEnglish(cron)).toBe(words);
  });
});

describe('cronToEnglish — falls back to the raw expression', () => {
  it.each([
    '*/30 * * * * *',      // sub-minute
    '0 0 9 * 1 *',         // month filter
    '0 0 9 L * *',         // last day of month
    '0 0 9 * * 1#2',       // nth weekday
    'not a cron',
    '0 0 9 * * * 2027',    // 7 fields
  ])('%s', (cron) => {
    expect(cronToEnglish(cron)).toBe(cron);
    expect(cronHasWords(cron)).toBe(false);
  });
});

describe('parseDayOfWeek', () => {
  it('treats 0 and 7 as Sunday', () => {
    expect([...parseDayOfWeek('0,7')!]).toEqual([0]);
  });
  it('rejects a reversed range', () => {
    expect(parseDayOfWeek('5-1')).toBeNull();
  });
});
