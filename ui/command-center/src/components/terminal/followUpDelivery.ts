/**
 * Follow-up prompt delivery — readiness-gated, not time-gated.
 *
 * `scheduleFollowUpInput` (removed) bracketed-pasted the directive on a BLIND
 * 2200 ms timer after the PTY spawned. On a Mac where the harness has to wait
 * on a Keychain prompt, a workspace-trust dialog, or MCP server startup, the
 * shell/harness is often still nowhere near raw mode at 2200 ms — the paste's
 * `ESC[200~…ESC[201~\r` lands on an ordinary line-buffered shell (or nothing
 * at all), the harness starts a moment later with an empty input line, and
 * the directive is gone. Reported: pressing "Send to Claude" on a Grow action
 * opened a terminal and started `claude` but never delivered the prompt.
 *
 * Fix: don't guess when the TUI is ready — watch for it. A TUI takes over the
 * tty by entering bracketed-paste mode (DEC private mode 2004) and/or the
 * alternate screen (1049); both are near-universal for interactive CLIs
 * (Claude Code, Codex, vim, less). This module tracks those modes live off
 * the PTY stream and only delivers once one of them is CURRENTLY set — not
 * "was set at some point" — because a harness that leaves the alt screen
 * (e.g. to print a line to the real scrollback) and re-enters it must be
 * treated as not-ready in between, or a delivery timed to the wrong dip lands
 * on bare shell output instead of the TUI's input box. A short settle timer
 * after the transition absorbs the few milliseconds most TUIs take between
 * "entered raw mode" and "drew an input box that will actually receive text."
 *
 * A ceiling guards the case where neither mode ever appears (the harness
 * exited early, or genuinely never enters bracketed-paste/alt-screen) — it
 * surfaces a "not delivered" chip so the user isn't left silently waiting.
 * Reaching the ceiling does not disarm delivery: if the TUI takes the tty
 * later, the prompt still lands and the chip clears.
 *
 * 2026-08-25: that "known limitation" turned out to be the common case, not an
 * edge one. "Send to Claude" from a Grow action reliably failed to deliver into
 * Claude Code while working into the Permagent harness. Captured PTY bytes
 * (see `harnessStartupFixtures.ts`) say why: `claude` sets bracketed paste at
 * byte 19 and then draws its workspace-trust dialog — "Is this a project you
 * created or one you trust?" — which is a yes/no selection, not an input box.
 * The gate fired on 2004h, the paste went to the dialog, and the prompt was
 * gone. `codex` sets 2004h at byte 0 with no such dialog, which is exactly why
 * codex worked.
 *
 * There IS a version-stable signal, it just is not 2004 alone. Bracketed paste
 * means "raw mode". What separates a dialog from an input box is whether the
 * harness went on to drive a full-screen surface:
 *   - alternate screen (1049) — Claude Code's prompt, vim, less
 *   - mouse tracking (1000/1002/1003/1006) — Claude Code's prompt
 *   - synchronized output (2026) — codex, which redraws in place instead
 * The trust dialog sets none of the three. So delivery now needs bracketed
 * paste AND one of those, or the alternate screen on its own.
 *
 * A harness that only ever sets 2004 and nothing else will now never
 * auto-deliver and will fall through to the 30 s chip. That is the intended
 * trade: a prompt the user can click "Send now" on beats a prompt silently
 * eaten by a dialog. Launching a harness in a directory it already trusts
 * skips the dialog entirely and delivers on the first transition.
 */

export const FOLLOW_UP_CEILING_MS = 30_000;
export const FOLLOW_UP_SETTLE_MS = 250;

// DEC private mode set/reset: ESC [ ? <params> h|l — matched globally so one
// chunk can carry several transitions (rare, but cheap to handle correctly).
const DEC_MODE_RE = /\x1b\[\?([0-9;]+)([hl])/g;

const BRACKETED_PASTE_MODE = 2004;
const ALT_SCREEN_MODE = 1049;

/**
 * Modes that mean the harness is driving a full-screen surface, not just
 * sitting in raw mode. Latched like any other DEC mode.
 */
const MOUSE_TRACKING_MODES = [1000, 1002, 1003, 1006];

/**
 * Synchronized output. Unlike the others this is INHERENTLY transient — a
 * harness wraps each redraw in 2026h/2026l, so "currently set" flickers many
 * times a second and is useless as a state test. Having seen one at all is the
 * signal: it means something is painting a UI.
 */
const SYNCHRONIZED_OUTPUT_MODE = 2026;

// Bound the cross-chunk carry: a real DEC mode sequence is short
// (`\x1b[?1049;2004h` is 14 chars), so anything longer than this cannot be
// the tail of one — cap it so a pathological chunk (no ESC at all, or one
// that never resolves) can't grow the carry without bound.
const MAX_CARRY = 32;

export interface FollowUpDelivery {
  /** Feed every PTY chunk, verbatim, in order. */
  onData(chunk: string): void;
  /** Deliver regardless of readiness (the "Send now" button). No-op once sent/cancelled. */
  sendNow(): void;
  /** Teardown: clear timers, never deliver afterwards. */
  cancel(): void;
}

export function createFollowUpDelivery(opts: {
  text: string;
  /** Writes the bracketed-paste payload to the PTY. */
  write: (data: string) => void;
  /** Ceiling reached without readiness — surface the prompt to the user. */
  onPending: () => void;
  /** Delivered — clear any pending UI. */
  onSent: () => void;
  ceilingMs?: number;
  settleMs?: number;
}): FollowUpDelivery {
  const { text, write, onPending, onSent } = opts;
  const ceilingMs = opts.ceilingMs ?? FOLLOW_UP_CEILING_MS;
  const settleMs = opts.settleMs ?? FOLLOW_UP_SETTLE_MS;

  const enabledModes = new Set<number>();
  let sawSynchronizedOutput = false;
  let carry = '';
  let sent = false;
  let cancelled = false;
  let settleTimer: ReturnType<typeof setTimeout> | null = null;
  let ceilingTimer: ReturnType<typeof setTimeout> | null = ceilingMs > 0 ? setTimeout(() => {
    ceilingTimer = null;
    if (sent || cancelled) return;
    console.warn('[terminal] follow-up prompt not delivered within %dms — showing pending chip', ceilingMs);
    onPending();
  }, ceilingMs) : null;

  const drivesFullScreenSurface = () =>
    enabledModes.has(ALT_SCREEN_MODE) ||
    MOUSE_TRACKING_MODES.some((m) => enabledModes.has(m)) ||
    sawSynchronizedOutput;

  // The alternate screen alone is proof enough — nothing takes it to show a
  // line of text. Otherwise raw mode has to be corroborated.
  const isActive = () =>
    enabledModes.has(ALT_SCREEN_MODE) ||
    (enabledModes.has(BRACKETED_PASTE_MODE) && drivesFullScreenSurface());

  const clearSettle = () => {
    if (settleTimer !== null) {
      clearTimeout(settleTimer);
      settleTimer = null;
    }
  };

  const deliver = () => {
    if (sent || cancelled) return;
    sent = true;
    clearSettle();
    if (ceilingTimer !== null) {
      clearTimeout(ceilingTimer);
      ceilingTimer = null;
    }
    write(`\x1b[200~${text}\x1b[201~\r`);
    // If the ceiling already fired, `onPending` put a chip up — clear it now
    // that delivery actually happened. If it never fired, this is a harmless
    // second "all clear" the caller is free to no-op on.
    onSent();
  };

  const armSettle = () => {
    clearSettle();
    settleTimer = setTimeout(() => {
      settleTimer = null;
      if (sent || cancelled) return;
      // Re-check: the TUI may have dropped out of raw mode again during the
      // settle window (a fast enter/exit blip) — only deliver if it is STILL
      // active right now.
      if (isActive()) deliver();
    }, settleMs);
  };

  const onData = (chunk: string) => {
    if (sent || cancelled) return;

    const wasActive = isActive();

    // Prepend whatever partial escape sequence the previous chunk ended on.
    const combined = carry + chunk;
    carry = '';

    DEC_MODE_RE.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = DEC_MODE_RE.exec(combined)) !== null) {
      const params = match[1].split(';').map(Number);
      const enable = match[2] === 'h';
      for (const p of params) {
        if (!Number.isFinite(p)) continue;
        if (p === SYNCHRONIZED_OUTPUT_MODE) {
          // Latch rather than track: see the constant's note.
          if (enable) sawSynchronizedOutput = true;
          continue;
        }
        if (enable) enabledModes.add(p);
        else enabledModes.delete(p);
      }
    }

    // Retain a possible partial escape sequence at the very end of this
    // chunk so a marker split across a chunk boundary is still recognized.
    const lastEsc = combined.lastIndexOf('\x1b');
    if (lastEsc !== -1) {
      const tail = combined.slice(lastEsc);
      // Only worth carrying if it looks like the start of an unterminated
      // DEC mode sequence (no 'h'/'l' after it yet) and is short enough to
      // plausibly still be in flight.
      if (tail.length <= MAX_CARRY && !/[hl]/.test(tail.slice(2))) {
        carry = tail;
      }
    }

    const nowActive = isActive();
    if (!wasActive && nowActive) {
      armSettle();
    } else if (wasActive && !nowActive) {
      // Re-arm: the TUI dropped out of raw mode before we delivered (alt-
      // screen exit, a harness redraw). Cancel the pending settle — the next
      // active transition schedules delivery again from scratch.
      clearSettle();
    }
  };

  return {
    onData,
    sendNow: () => deliver(),
    cancel: () => {
      if (cancelled || sent) return;
      cancelled = true;
      clearSettle();
      if (ceilingTimer !== null) {
        clearTimeout(ceilingTimer);
        ceilingTimer = null;
      }
    },
  };
}
