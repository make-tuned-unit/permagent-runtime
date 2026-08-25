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

export function usePollWhenVisible(callback: () => void, intervalMs: number) {
  const cbRef = useRef(callback);
  cbRef.current = callback;

  useEffect(() => {
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
      start();
    }
    document.addEventListener('visibilitychange', handleVisibilityChange);

    return () => {
      stop();
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [intervalMs]);
}
