/**
 * Shared schedule-change subscription primitive.
 *
 * The ONE place that subscribes to the daemon `/events` stream for schedule
 * lifecycle changes (`schedule_changed`, emitted by the `/schedule/*` route
 * handlers on create/update/delete/pause/unpause/run_now — see
 * `permagent::events::schedule_changed`). Added for the 2026-08-25 "schedule
 * polling storm" health-review fix: the Automate tab used to be the only
 * source of freshness, polling `/schedule/list` every 5s indefinitely. Now
 * a real write pushes an event and the tab refetches immediately; the poll
 * in `AutomateView` is just a >=60s backstop for whatever this stream
 * misses (a dropped connection, a cron-triggered run with no route call).
 *
 * Mirrors `useGoalEvents.ts`'s proven reconnect logic exactly — same
 * WebSocket lifecycle, same "skip buffered replay events from before mount"
 * rule.
 */

import { useEffect, useRef } from 'react';
import { eventsWsUrl } from './api';
import { wireEventType } from './wireEvent';

export function useScheduleEvents(onScheduleChange: () => void) {
  const cbRef = useRef(onScheduleChange);
  cbRef.current = onScheduleChange;

  useEffect(() => {
    let ws: WebSocket | null = null;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let closed = false;
    const mountedAt = Date.now();

    const connect = async () => {
      if (closed) return;
      // Daemon token rides the WS query (C1/C2 auth); re-check `closed` after
      // the await so a token load racing unmount never opens an orphan socket.
      const url = await eventsWsUrl();
      if (closed) return;
      try {
        ws = new WebSocket(url);
      } catch {
        return;
      }
      ws.onmessage = (ev) => {
        let parsed: { type?: string; event_type?: string; timestamp?: string };
        try {
          parsed = JSON.parse(ev.data);
        } catch {
          return;
        }
        const ts = Date.parse(parsed.timestamp ?? '');
        if (Number.isFinite(ts) && ts < mountedAt) return; // skip replayed buffer
        if (wireEventType(parsed) === 'schedule_changed') {
          cbRef.current();
        }
      };
      ws.onerror = () => ws?.close();
      ws.onclose = () => {
        if (!closed) retry = setTimeout(connect, 3000);
      };
    };
    connect();

    return () => {
      closed = true;
      if (retry) clearTimeout(retry);
      ws?.close();
      ws = null;
    };
  }, []);
}
