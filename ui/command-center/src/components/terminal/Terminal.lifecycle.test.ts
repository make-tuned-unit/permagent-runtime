import { afterEach, expect, it, vi } from 'vitest';
import { scheduleFollowUpInput, scheduleInitialCommand } from './Terminal';

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

it('cancels a delayed follow-up paste during teardown', async () => {
  vi.useFakeTimers();
  const invoke = vi.fn().mockResolvedValue(undefined);
  const cancel = scheduleFollowUpInput(invoke, 'pty-1', 'do the thing');

  cancel();
  await vi.advanceTimersByTimeAsync(2200);

  expect(invoke).not.toHaveBeenCalled();
});
