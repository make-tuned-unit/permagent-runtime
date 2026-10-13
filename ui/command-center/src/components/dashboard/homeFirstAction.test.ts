import { describe, expect, it } from 'vitest';
import { isUntouchedHomeData } from './homeFirstAction';

describe('isUntouchedHomeData', () => {
  const base = { stats: { sessions_total: 0, memory_count: 0 }, recent: [], in_flight: [] };
  it('is untouched when the daemon reports no sessions with messages and no user memories', () => {
    expect(isUntouchedHomeData(base)).toBe(true);
  });
  it('is not untouched once a session has messages, a memory exists, or work is listed', () => {
    expect(isUntouchedHomeData({ ...base, stats: { sessions_total: 1, memory_count: 0 } })).toBe(false);
    expect(isUntouchedHomeData({ ...base, stats: { sessions_total: 0, memory_count: 2 } })).toBe(false);
    expect(isUntouchedHomeData({ ...base, recent: [{}] })).toBe(false);
    expect(isUntouchedHomeData({ ...base, in_flight: [{}] })).toBe(false);
  });
});
