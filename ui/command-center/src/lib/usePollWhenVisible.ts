/**
 * Runs `callback` on a fixed interval, but ONLY while the document is
 * visible (`document.visibilityState === 'visible'`) — a backgrounded or
 * minimized tab burns no daemon cycles. Fires `callback` once immediately
 * when the tab regains visibility (catching up on whatever it missed while
 * hidden), then resumes the interval. The interval is torn down on unmount
 * and whenever the tab goes hidden, so nothing fires for an unmounted or
 * hidden consumer.
 *
 * Extracted for the Automate tab's `/schedule/list` poll (the 2026-08-25
 * "schedule polling storm" health review: a 5s indefinite poll hammering an
 * unindexed SQL query, 97 slow-query bursts in 9 minutes). Live updates now
 * come from `useScheduleEvents`'s SSE-equivalent WebSocket subscription —
 * this hook is only the backstop for whatever that stream misses.
 */

import { useEffect, useRef } from 'react';

/**
 * @param enabled Poll at all. `false` tears the interval down exactly as
 *   hiding the tab does — for consumers that stay mounted while off screen
 *   (the app hides inactive workspaces rather than unmounting them, so
 *   `document.visibilityState` alone would keep every tab polling at once),
 *   and for the window in which a live event stream is already covering it.
 *   Flipping it back to `true` fires the callback once immediately, the same
 *   catch-up a tab gets when it regains visibility.
 */
export function usePollWhenVisible(callback: () => void, intervalMs: number, enabled = true) {
  const cbRef = useRef(callback);
  cbRef.current = callback;
  // Seeded from the first `enabled` so mounting enabled is not itself treated
  // as a re-enable: callers already do their own initial load.
  const wasEnabled = useRef(enabled);

  useEffect(() => {
    if (!enabled) { wasEnabled.current = false; return; }
    const reEnabled = !wasEnabled.current;
    wasEnabled.current = true;
    let interval: ReturnType<typeof setInterval> | undefined;

    const start = () => {
      if (interval !== undefined) return;
      interval = setInterval(() => cbRef.current(), intervalMs);
    };
    const stop = () => {
      if (interval !== undefined) {
        clearInterval(interval);
        interval = undefined;
      }
    };

    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        cbRef.current();
        start();
      } else {
        stop();
      }
    };

    if (document.visibilityState === 'visible') {
      // Coming back on screen is the same catch-up as a tab regaining
      // visibility — whatever landed while we were away lands now.
      if (reEnabled) cbRef.current();
      start();
    }
    document.addEventListener('visibilitychange', handleVisibilityChange);

    return () => {
      stop();
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [intervalMs, enabled]);
}
