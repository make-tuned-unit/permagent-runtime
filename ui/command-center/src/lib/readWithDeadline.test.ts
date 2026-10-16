import { afterEach, expect, it, vi } from 'vitest';
import { readWithDeadline } from './readWithDeadline';
afterEach(() => { vi.useRealTimers(); });
it('bounds a read that ignores cancellation and aborts the transport', async () => {
  vi.useFakeTimers();
  let signal!: AbortSignal;
  const read = readWithDeadline(s => { signal = s; return new Promise(() => {}); }, 100);
  const result = expect(read).rejects.toThrow('timed out');
  await vi.advanceTimersByTimeAsync(100);
  await result;
  expect(signal.aborted).toBe(true);
  expect(vi.getTimerCount()).toBe(0);
});
it('returns successful data and removes its deadline', async () => {
  vi.useFakeTimers();
  expect(await readWithDeadline(async () => ({ count: 4 }))).toEqual({ count: 4 });
  expect(vi.getTimerCount()).toBe(0);
});
it('preserves transport failures', async () => {
  await expect(readWithDeadline(async () => { throw new Error('offline'); })).rejects.toThrow('offline');
});
