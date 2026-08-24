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

const EXPECTED_PAYLOAD = `\x1b[200~do the thing\x1b[201~\r`;

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

  it('delivers the exact payload after 2004 (bracketed paste) + settle', () => {
    const { delivery, writes, sentCalls } = make();
    delivery.onData('\x1b[?2004h');
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
    delivery.onData('\x1b[?20');
    delivery.onData('04h');
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

    delivery.onData('\x1b[?2004h');
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

    delivery.onData('\x1b[?2004h');
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
