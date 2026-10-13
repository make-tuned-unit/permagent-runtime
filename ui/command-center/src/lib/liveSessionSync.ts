/**
 * Live sync of the open chat pane with turns made on ANOTHER device.
 *
 * Why this exists: the pane streams only its own turns (the per-session SSE
 * bus fed by POST /sessions/{id}/reply). A turn the Watch/iPhone relay makes
 * with POST /reply on the same session id publishes nothing the pane hears,
 * and the daemon's global `/events` socket has no per-message frame (see
 * `message_received`: defined, never emitted). Adding one is a daemon change;
 * until then the pane re-reads the session from the hub on a slow poll.
 *
 * The poll is deliberately light: one GET per interval, only while a session
 * is open, the pane is idle (not streaming), and the page is visible. An
 * unchanged conversation re-renders nothing (see `loadSessionMessages`'s
 * quiet mode); a changed one replaces the daemon-derived messages, and
 * MessageList's own autoscroll keeps a reader who scrolled up where they are.
 */
import { useEffect } from 'react';
import { useCommandCenter } from './store';

export const LIVE_SESSION_POLL_MS = 10_000;

/** One poll tick; exported for tests. Resolves once the read settled. */
export async function syncOpenSessionOnce(): Promise<void> {
  const s = useCommandCenter.getState();
  const sid = s.chatSessionId;
  if (!sid || s.isStreaming || !s.chatHistoryLoaded) return;
  if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
  await s.loadSessionMessages(sid, { quiet: true });
}

/** Mount once per chat surface (dock and pop-out window). */
export function useLiveSessionSync(): void {
  useEffect(() => {
    const id = setInterval(() => { void syncOpenSessionOnce(); }, LIVE_SESSION_POLL_MS);
    const onVisible = () => { void syncOpenSessionOnce(); };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      clearInterval(id);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, []);
}
