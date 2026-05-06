import { describe, it, expect, vi, afterEach } from 'vitest';
import { relativeTimeAgo } from './time-decay';

describe('relativeTimeAgo', () => {
  afterEach(() => { vi.useRealTimers(); });

  function fixed(minutesAgo: number): Date {
    return new Date(Date.now() - minutesAgo * 60_000);
  }

  it('returns empty string for now', () => {
    expect(relativeTimeAgo(new Date())).toBe('');
  });

  it('returns empty string for 4 minutes ago', () => {
    expect(relativeTimeAgo(fixed(4))).toBe('');
  });

  it('returns "6 min ago" for 6 minutes', () => {
    expect(relativeTimeAgo(fixed(6))).toBe('6 min ago');
  });

  it('returns "1h 5m ago" for 65 minutes', () => {
    expect(relativeTimeAgo(fixed(65))).toBe('1h 5m ago');
  });

  it('returns "2h 5m ago" for 125 minutes', () => {
    expect(relativeTimeAgo(fixed(125))).toBe('2h 5m ago');
  });

  it('returns "1h ago" for exactly 60 minutes', () => {
    expect(relativeTimeAgo(fixed(60))).toBe('1h ago');
  });

  it('accepts ISO string timestamps', () => {
    const ts = fixed(30).toISOString();
    expect(relativeTimeAgo(ts)).toBe('30 min ago');
  });
});
