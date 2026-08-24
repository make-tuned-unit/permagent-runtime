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

// The blind-timer follow-up paste (scheduleFollowUpInput) is gone — it wrote
// on a fixed 2200ms clock with no idea whether the TUI had actually taken the
// tty, which is why the prompt could land nowhere. Its teardown coverage is
// replaced by followUpDelivery.test.ts (the readiness state machine's
// cancel() case) and Terminal.followUp.test.tsx (wired into the component).
