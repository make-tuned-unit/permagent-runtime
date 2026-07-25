import { afterEach, expect, it, vi } from 'vitest';
import { scheduleInitialCommand } from './Terminal';

afterEach(() => {
  vi.useRealTimers();
});

it('cancels a delayed initial PTY command during teardown', async () => {
  vi.useFakeTimers();
  const invoke = vi.fn().mockResolvedValue(undefined);
  const cancel = scheduleInitialCommand(invoke, 'pty-1', 'codex');

  cancel();
  await vi.advanceTimersByTimeAsync(300);

  expect(invoke).not.toHaveBeenCalled();
});
