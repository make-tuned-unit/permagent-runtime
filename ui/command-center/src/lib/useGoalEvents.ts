/**
 * Shared goal-state subscription primitive.
 *
 * The ONE place that subscribes to the daemon `/events` stream for goal
 * lifecycle changes (`goal_state_changed`, emitted by the goal-transition guard
 * + goal create/delete). Every goal surface — the dashboard's unified "in
 * flight" hook and the Kanban board — routes through this instead of opening its
 * own ad-hoc WebSocket. Mirrors the proven reconnect logic in useDecisions.ts.
 *
 * `onGoalChange` is invoked (debounced to the next tick is unnecessary — the
 * consumers refetch) whenever a goal event arrives after mount; buffered replay
 * events from before mount are skipped.
 */

import { useEffect, useRef } from 'react';
import { eventsWsUrl } from './api';
import { wireEventType } from './wireEvent';

export function useGoalEvents(onGoalChange: () => void) {
  const cbRef = useRef(onGoalChange);
  cbRef.current = onGoalChange;

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
        if (wireEventType(parsed) === 'goal_state_changed') {
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
