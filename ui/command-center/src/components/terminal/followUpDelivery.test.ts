/**
 * followUpDelivery — readiness state machine that replaced the blind 2200ms
 * timer (see followUpDelivery.ts for the failure this prevents). Pure logic,
 * fake timers, no DOM.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createFollowUpDelivery,
  FOLLOW_UP_CEILING_MS,
  FOLLOW_UP_SETTLE_MS,
  type FollowUpDelivery,
} from './followUpDelivery';
import {
  claudeReadyPromptBytes,
  claudeTrustDialogBytes,
  codexReadyPromptBytes,
} from './harnessStartupFixtures';

const EXPECTED_PAYLOAD = `\x1b[200~do the thing\x1b[201~\r`;

/**
 * A harness at its input box: bracketed paste plus a full-screen surface.
 *
 * Bracketed paste ALONE stopped counting as ready on 2026-08-25 — Claude Code
 * sets it for its workspace-trust dialog too, and a paste delivered there is
 * an answer to that dialog rather than a prompt (see followUpDelivery.ts and
 * the fixture-driven tests at the bottom of this file).
 */
const READY_MARKER = '\x1b[?2004h\x1b[?1049h';

function make(overrides?: Partial<Parameters<typeof createFollowUpDelivery>[0]>) {
  const writes: string[] = [];
  const pendingCalls: number[] = [];
  const sentCalls: number[] = [];
  const delivery = createFollowUpDelivery({
    text: 'do the thing',
    write: (d) => writes.push(d),
    onPending: () => pendingCalls.push(1),
    onSent: () => sentCalls.push(1),
    ...overrides,
  });
  return { delivery, writes, pendingCalls, sentCalls };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('createFollowUpDelivery', () => {
  it('writes nothing before any readiness marker arrives', () => {
    const { writes } = make();
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS - 1);
    expect(writes).toEqual([]);
  });

  it('delivers the exact payload once the harness is at its input box', () => {
    const { delivery, writes, sentCalls } = make();
    delivery.onData(READY_MARKER);
    expect(writes).toEqual([]); // not yet — settle hasn't elapsed
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
    expect(sentCalls).toEqual([1]);
  });

  it('delivers after 1049 (alt screen) + settle', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[?1049h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  it('handles a combined-param sequence (1049;2004h) as one activation', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[?1049;2004h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  it('recognizes a marker split across two chunks', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[?2004h\x1b[?10');
    delivery.onData('49h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  /**
   * The regression this whole change exists to prevent. Bracketed paste on its
   * own is raw mode, not an input box — Claude Code sets it to draw its
   * workspace-trust dialog.
   */
  it('does NOT deliver on bracketed paste alone', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[?2004h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS * 4);
    expect(writes).toEqual([]);
  });

  it('accepts mouse tracking as the corroborating surface signal', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[?2004h\x1b[?1006h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  /**
   * Synchronized output is a begin/end pair around every redraw, so it is
   * latched on sighting rather than tracked as state — by the time the settle
   * timer fires the harness has long since sent 2026l.
   */
  it('accepts synchronized output even though it is already switched off again', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[?2004h\x1b[?2026h\x1b[?2026l');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  it('re-arms: an alt-screen exit before settle cancels delivery, and a later re-enter still delivers', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[?1049h');
    // Exit before the settle timer fires.
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS - 50);
    delivery.onData('\x1b[?1049l');
    vi.advanceTimersByTime(50);
    expect(writes).toEqual([]); // the cancelled settle must not have delivered

    // Re-enter later — this must schedule a fresh settle and deliver.
    delivery.onData('\x1b[?1049h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  it('ceiling fires onPending and writes nothing when no marker ever arrives', () => {
    const { writes, pendingCalls } = make();
    vi.advanceTimersByTime(FOLLOW_UP_CEILING_MS);
    expect(writes).toEqual([]);
    expect(pendingCalls).toEqual([1]);
  });

  it('a late marker after the ceiling still delivers and calls onSent', () => {
    const { delivery, writes, pendingCalls, sentCalls } = make();
    vi.advanceTimersByTime(FOLLOW_UP_CEILING_MS);
    expect(pendingCalls).toEqual([1]);

    delivery.onData(READY_MARKER);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
    expect(sentCalls).toEqual([1]);
  });

  it('sendNow() writes exactly once, and a later marker does not write again', () => {
    const { delivery, writes, sentCalls } = make();
    delivery.sendNow();
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
    expect(sentCalls).toEqual([1]);

    delivery.onData('\x1b[?2004h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]); // still just the one
  });

  it('sendNow() after cancel() is a no-op', () => {
    const { delivery, writes } = make();
    delivery.cancel();
    delivery.sendNow();
    expect(writes).toEqual([]);
  });

  it('cancel() prevents delivery from a marker already in flight', () => {
    const { delivery, writes, pendingCalls } = make();
    delivery.onData('\x1b[?2004h');
    delivery.cancel();
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    vi.advanceTimersByTime(FOLLOW_UP_CEILING_MS);
    expect(writes).toEqual([]);
    expect(pendingCalls).toEqual([]); // ceiling must not fire post-cancel either
  });

  it('cancel() before any marker suppresses the ceiling too', () => {
    const { delivery, pendingCalls } = make();
    delivery.cancel();
    vi.advanceTimersByTime(FOLLOW_UP_CEILING_MS);
    expect(pendingCalls).toEqual([]);
  });

  it('writes the exact bracketed-paste payload', () => {
    const { delivery, writes } = make({ text: 'hello\nworld' });
    delivery.sendNow();
    expect(writes).toEqual([`\x1b[200~hello\nworld\x1b[201~\r`]);
  });

  it('onData is a cheap no-op after delivery (no throw, no further writes)', () => {
    const { delivery, writes } = make();
    delivery.sendNow();
    expect(() => delivery.onData('\x1b[?1049h')).not.toThrow();
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  it('honors custom ceilingMs/settleMs overrides', () => {
    const { delivery, writes, pendingCalls } = make({ ceilingMs: 1000, settleMs: 50 });
    vi.advanceTimersByTime(999);
    expect(pendingCalls).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(pendingCalls).toEqual([1]);

    delivery.onData(READY_MARKER);
    vi.advanceTimersByTime(49);
    expect(writes).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  it('type-level: the exported factory returns a FollowUpDelivery', () => {
    const { delivery } = make();
    const typed: FollowUpDelivery = delivery;
    expect(typeof typed.onData).toBe('function');
  });
});

/**
 * The reported bug, driven by the bytes the real harnesses actually emit.
 * Captured on this Mac on 2026-08-25; see harnessStartupFixtures.ts.
 */
describe('real harness startup streams', () => {
  it('does NOT deliver into Claude Code\'s workspace-trust dialog', () => {
    const { delivery, writes } = make();
    delivery.onData(claudeTrustDialogBytes);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS * 4);
    expect(writes).toEqual([]);
  });

  it('shows the pending chip rather than silently eating the prompt at the dialog', () => {
    const { delivery, writes, pendingCalls } = make();
    delivery.onData(claudeTrustDialogBytes);
    vi.advanceTimersByTime(FOLLOW_UP_CEILING_MS + 1);
    expect(writes).toEqual([]);
    expect(pendingCalls).toHaveLength(1);
  });

  it('delivers once Claude Code reaches its input box', () => {
    const { delivery, writes } = make();
    delivery.onData(claudeReadyPromptBytes);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  /**
   * The exact live sequence: the dialog first, then the user answers it and
   * Claude draws the prompt. Nothing may be written until the second half.
   */
  it('waits through the dialog and delivers after it is answered', () => {
    const { delivery, writes } = make();
    delivery.onData(claudeTrustDialogBytes);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS * 4);
    expect(writes).toEqual([]);

    delivery.onData(claudeReadyPromptBytes);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  /**
   * codex never takes the alternate screen and never turns on mouse tracking —
   * it redraws in place under synchronized output. It must keep working, which
   * is why the gate accepts 2026 as well.
   */
  it('still delivers into codex, which drives no alternate screen', () => {
    const { delivery, writes } = make();
    delivery.onData(codexReadyPromptBytes);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  /**
   * Byte-boundary safety: the same stream fed one byte at a time must reach the
   * same verdict as one big chunk.
   */
  it('reaches the same verdict when the stream is split byte by byte', () => {
    const dialog = make();
    for (const byte of claudeTrustDialogBytes) dialog.delivery.onData(byte);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS * 4);
    expect(dialog.writes).toEqual([]);

    const ready = make();
    for (const byte of claudeReadyPromptBytes) ready.delivery.onData(byte);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(ready.writes).toEqual([EXPECTED_PAYLOAD]);
  });

  it('"Send now" still overrides the dialog, for the user who wants it there', () => {
    const { delivery, writes } = make();
    delivery.onData(claudeTrustDialogBytes);
    delivery.sendNow();
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });
});
