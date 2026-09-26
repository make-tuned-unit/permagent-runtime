import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

vi.mock('../../lib/api', () => ({
  api: { forgetBrainMemory: vi.fn() },
}));

import { api } from '../../lib/api';
import { scheduleForget, FORGET_UNDO_MS } from './brainForget';

describe('scheduleForget', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.mocked(api.forgetBrainMemory).mockReset();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('sends the DELETE only after the undo window expires', async () => {
    vi.mocked(api.forgetBrainMemory).mockResolvedValue({ existed: true, verified: true });
    const onSettled = vi.fn();
    scheduleForget('note:abc', onSettled);

    expect(api.forgetBrainMemory).not.toHaveBeenCalled();
    vi.advanceTimersByTime(FORGET_UNDO_MS - 1);
    expect(api.forgetBrainMemory).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    // Let the resolved promise's .then() microtask run.
    await vi.runOnlyPendingTimersAsync();
    expect(api.forgetBrainMemory).toHaveBeenCalledWith('note:abc');
    expect(onSettled).toHaveBeenCalledWith('deleted');
  });

  it('cancelling before expiry sends no request at all (Undo)', async () => {
    const onSettled = vi.fn();
    const cancel = scheduleForget('note:abc', onSettled);
    vi.advanceTimersByTime(FORGET_UNDO_MS / 2);
    cancel();
    await vi.advanceTimersByTimeAsync(FORGET_UNDO_MS * 2);

    expect(api.forgetBrainMemory).not.toHaveBeenCalled();
    expect(onSettled).not.toHaveBeenCalled();
  });

  it('reports "restored" when the request itself fails, without throwing', async () => {
    vi.mocked(api.forgetBrainMemory).mockRejectedValue(new Error('network down'));
    const onSettled = vi.fn();
    scheduleForget('note:abc', onSettled);

    await vi.advanceTimersByTimeAsync(FORGET_UNDO_MS + 1);
    expect(onSettled).toHaveBeenCalledWith('restored');
  });

  it('a cancel() called after the request already fired is a harmless no-op', async () => {
    vi.mocked(api.forgetBrainMemory).mockResolvedValue({ existed: true, verified: true });
    const onSettled = vi.fn();
    const cancel = scheduleForget('note:abc', onSettled);
    await vi.advanceTimersByTimeAsync(FORGET_UNDO_MS + 1);
    expect(onSettled).toHaveBeenCalledWith('deleted');
    expect(() => cancel()).not.toThrow();
    expect(api.forgetBrainMemory).toHaveBeenCalledTimes(1);
  });
});
