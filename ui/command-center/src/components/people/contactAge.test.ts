import { describe, expect, it } from 'vitest';
import { contactLabel, isFollowUpDue, isQuiet, QUIET_AFTER_DAYS } from './contactAge';

const NOW = Date.parse('2026-08-20T12:00:00Z');

describe('contactAge', () => {
  it('treats missing and old contact as quiet', () => {
    expect(isQuiet(null, NOW)).toBe(true);
    expect(isQuiet('2026-07-01T12:00:00Z', NOW)).toBe(true);
    expect(isQuiet('2026-08-18T12:00:00Z', NOW)).toBe(false);
    expect(QUIET_AFTER_DAYS).toBe(30);
  });

  it('labels never / today / weeks', () => {
    expect(contactLabel(null, NOW)).toBe('never');
    expect(contactLabel('2026-08-20T09:00:00Z', NOW)).toBe('today');
    expect(contactLabel('2026-08-19T12:00:00Z', NOW)).toBe('yesterday');
    expect(contactLabel('2026-08-10T12:00:00Z', NOW)).toBe('10d ago');
  });

  it('flags an overdue follow-up', () => {
    expect(isFollowUpDue('2026-08-19T12:00:00Z', NOW)).toBe(true);
    expect(isFollowUpDue('2026-08-21T12:00:00Z', NOW)).toBe(false);
    expect(isFollowUpDue(null, NOW)).toBe(false);
  });
});
