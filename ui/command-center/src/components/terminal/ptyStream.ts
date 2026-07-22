/**
 * PTY stream discipline (#573).
 *
 * The terminal pane has exactly ONE writer: the PTY byte stream, rendered
 * verbatim. Nothing in the frontend may inject characters into the xterm
 * buffer or mutate the stream in flight.
 *
 * History: #239 added a "local echo" fast path (frontend wrote printable
 * keystrokes straight into xterm, then stripped the matching leading bytes
 * from the next PTY chunk). That was only sound for canonical-mode input,
 * which no modern interactive tool uses — zsh's ZLE and TUI apps (Claude
 * Code/Ink) run the PTY raw with termios ECHO off and repaint themselves.
 * Because the Rust reader reports ECHO transitions only (starting from
 * `false`), a normal zsh → claude flow never emitted `pty_echo_state`, so
 * the frontend's `echoEnabled = true` default kept local echo active during
 * TUI sessions. Two artifacts followed (#573):
 *
 *  1. A locally echoed char landed at the TUI's cursor (end of its frame);
 *     at the last column it wrapped the line, shifting the frame down one
 *     row — the TUI's next cursor-up repaint was then off by one, leaving a
 *     stale status line behind (the doubled "Forging…" ghost).
 *  2. Un-consumed pending-echo entries lived for 2s and stripped legitimate
 *     leading bytes from later PTY chunks, corrupting repaint/erase
 *     sequences mid-frame (the "messy overlap during active jobs").
 *
 * The latency #239 targeted was actually fixed by its other half (targeted
 * `emit_to("main")` instead of broadcasting to every webview); a localhost
 * PTY round-trip is ~1ms, so local echo bought nothing and cost render
 * correctness. It is removed; this module pins the verbatim contract.
 */

export interface PtyDataPayload {
  session_id: string;
  data: string;
}

export interface PtyStreamSink {
  /** Write PTY bytes to the terminal — must be called with the payload verbatim. */
  write: (data: string) => void;
  /** OSC 7 working-directory report (decoded path). */
  onCwd?: (path: string) => void;
}

const OSC7_RE = /\x1b\]7;file:\/\/[^/]*([^\x07\x1b]+)/;

/**
 * Handle one `pty_data` event: filter by session, forward the bytes to the
 * terminal unmodified, and surface any OSC 7 CWD report.
 */
export function handlePtyData(
  payload: PtyDataPayload,
  currentSessionId: string | null,
  sink: PtyStreamSink,
): void {
  if (payload.session_id !== currentSessionId) return;
  if (payload.data.length > 0) {
    sink.write(payload.data);
  }
  const osc7 = payload.data.match(OSC7_RE);
  if (osc7) {
    try {
      sink.onCwd?.(decodeURIComponent(osc7[1]));
    } catch {
      /* ignore decode errors */
    }
  }
}
