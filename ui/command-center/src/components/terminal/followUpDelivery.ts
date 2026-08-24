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
 * Known limitation: a blocking dialog (e.g. a first-run trust prompt) that
 * appears AFTER the harness has already turned on bracketed paste will still
 * receive the delivered paste — there is no version-stable signal that
 * distinguishes "TUI is showing its input box" from "TUI is showing a raw-mode
 * dialog" across harness versions, so this is not something worth chasing.
 * The ceiling chip's "Send now" / "Copy" buttons are the user-visible recovery
 * for that case.
 */

export const FOLLOW_UP_CEILING_MS = 30_000;
export const FOLLOW_UP_SETTLE_MS = 250;

// DEC private mode set/reset: ESC [ ? <params> h|l — matched globally so one
// chunk can carry several transitions (rare, but cheap to handle correctly).
const DEC_MODE_RE = /\x1b\[\?([0-9;]+)([hl])/g;

const BRACKETED_PASTE_MODE = 2004;
const ALT_SCREEN_MODE = 1049;

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

  const isActive = () => enabledModes.has(BRACKETED_PASTE_MODE) || enabledModes.has(ALT_SCREEN_MODE);

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
