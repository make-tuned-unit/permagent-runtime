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
  permagentReadyComposerBytes,
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
    write: (d) => { writes.push(d); },
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

  it('waits for an asynchronous PTY write before acknowledging delivery', async () => {
    let resolveWrite!: () => void;
    const write = vi.fn(() => new Promise<void>((resolve) => { resolveWrite = resolve; }));
    const { delivery, sentCalls } = make({ write });

    delivery.sendNow();
    expect(write).toHaveBeenCalledTimes(1);
    expect(sentCalls).toEqual([]);
    // Repeated clicks while the native write is pending cannot duplicate it.
    delivery.sendNow();
    expect(write).toHaveBeenCalledTimes(1);

    resolveWrite();
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(sentCalls).toEqual([1]);
  });

  it('keeps a failed asynchronous write pending and permits an explicit retry', async () => {
    const write = vi.fn()
      .mockRejectedValueOnce(new Error('closed PTY'))
      .mockResolvedValueOnce(undefined);
    const { delivery, pendingCalls, sentCalls } = make({ write });

    delivery.sendNow();
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(pendingCalls).toEqual([1]);
    expect(sentCalls).toEqual([]);

    delivery.sendNow();
    expect(write).toHaveBeenCalledTimes(2);
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(sentCalls).toEqual([1]);
  });

  it('does not retry an ambiguously failed readiness write on later PTY output', async () => {
    const write = vi.fn()
      .mockRejectedValueOnce(new Error('closed PTY'))
      .mockResolvedValueOnce(undefined);
    const { delivery, pendingCalls, sentCalls } = make({ write });

    delivery.onData(READY_MARKER);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(write).toHaveBeenCalledTimes(1);
    expect(pendingCalls).toEqual([1]);

    // A later TUI exit and re-entry is another readiness edge, but it must not
    // automatically repeat a paste whose first write may have been partial.
    delivery.onData('\x1b[?1049l');
    delivery.onData(READY_MARKER);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(write).toHaveBeenCalledTimes(1);
    expect(sentCalls).toEqual([]);

    delivery.sendNow();
    expect(write).toHaveBeenCalledTimes(2);
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(sentCalls).toEqual([1]);
  });

  it('does not acknowledge a write that resolves after cancellation', async () => {
    let resolveWrite!: () => void;
    const write = vi.fn(() => new Promise<void>((resolve) => { resolveWrite = resolve; }));
    const { delivery, sentCalls, pendingCalls } = make({ write });

    delivery.sendNow();
    delivery.cancel();
    resolveWrite();
    for (let i = 0; i < 4; i++) await Promise.resolve();
    expect(sentCalls).toEqual([]);
    expect(pendingCalls).toEqual([]);
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

  /**
   * A pinned-strip harness (the Permagent composer) never takes the alternate
   * screen and never turns on mouse tracking — it carves the bottom rows out
   * of the scrolling region with DECSTBM and paints there. The scroll region
   * IS the full-screen surface in that design, so it has to corroborate raw
   * mode like the others do.
   */
  it('accepts a DECSTBM scroll region as the corroborating surface signal', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[1;19r\x1b[19;1H\x1b[?2004h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  /**
   * `ESC [ r` with no parameters resets the region to the whole screen —
   * Claude Code emits exactly that at startup, before its trust dialog. It
   * reserves nothing, so it must not stand in for a pinned strip.
   */
  it('does NOT treat a parameterless DECSTBM reset as a surface', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b7\x1b[r\x1b8\x1b[?2004h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS * 4);
    expect(writes).toEqual([]);
  });

  it('drops back to not-ready when the harness releases its scroll region', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[1;19r\x1b[?2004h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS - 50);
    // Suspend: the composer tears the strip down (2004l then a bare DECSTBM).
    delivery.onData('\x1b[?2004l\x1b[r');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([]);

    delivery.onData('\x1b[1;19r\x1b[?2004h');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  it('recognizes a DECSTBM split across two chunks', () => {
    const { delivery, writes } = make();
    delivery.onData('\x1b[?2004h\x1b[1;1');
    delivery.onData('9r');
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
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
  /**
   * The reported bug (screenshot 2026-09-18): a Grow action sent to the
   * Permagent harness sat out the full 30 s ceiling and surfaced the "Prompt
   * not delivered — the agent didn't take the terminal in time." chip, while
   * the terminal behind it was plainly at "• Ready · enter send · ctrl+j
   * newline · /help". These bytes are that terminal.
   */
  it('delivers into the Permagent harness at its pinned composer', () => {
    const { delivery, writes } = make();
    delivery.onData(permagentReadyComposerBytes);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  it('never reaches the ceiling for the Permagent harness', () => {
    const { delivery, writes, pendingCalls } = make();
    delivery.onData(permagentReadyComposerBytes);
    vi.advanceTimersByTime(FOLLOW_UP_CEILING_MS + 1);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
    expect(pendingCalls).toEqual([]);
  });

  it('delivers into the Permagent harness byte by byte too', () => {
    const { delivery, writes } = make();
    for (const byte of permagentReadyComposerBytes) delivery.onData(byte);
    vi.advanceTimersByTime(FOLLOW_UP_SETTLE_MS);
    expect(writes).toEqual([EXPECTED_PAYLOAD]);
  });

  /**
   * Pins what the capture actually proves, so nobody re-derives readiness from
   * a signal this harness does not emit: the ONLY DEC private modes in it are
   * bracketed paste and cursor visibility. Alt screen, mouse tracking and
   * synchronized output are all absent — DECSTBM is the whole signal.
   */
  it('the Permagent capture carries no alt screen, mouse tracking or sync output', () => {
    const modes = new Set(
      [...permagentReadyComposerBytes.matchAll(/\x1b\[\?([0-9;]+)[hl]/g)].flatMap((m) =>
        m[1].split(';'),
      ),
    );
    expect([...modes].sort()).toEqual(['2004', '25']);
    expect(permagentReadyComposerBytes).toContain('\x1b[1;19r');
  });
});
