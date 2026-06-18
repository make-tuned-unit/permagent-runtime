/**
 * Decision Inbox — data hook (Lane L4).
 *
 * Event-driven via the daemon /events stream (#302): decision_created /
 * decision_resolved trigger an immediate refresh so the board reacts within
 * a frame instead of waiting up to 15s. The 15s poll stays as a backstop for
 * missed events / reconnect gaps.
 */

import { useState, useEffect, useRef, useCallback } from 'react';
import { getApiBaseUrl } from '../../../lib/api';
import { decisionsClient } from './client';
import type { AnswerBody, Decision, DecisionsResponse, HistoryItem } from './types';
import { DecisionConflictError } from './types';

export type AnswerResult =
  | { ok: true; decision: Decision; effect: string | null }
  | { ok: false; conflict: true };

export function useDecisions() {
  const [data, setData] = useState<DecisionsResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const showAllRef = useRef(false);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();

  const fetchDecisions = useCallback(async () => {
    try {
      const result = await decisionsClient.list(
        showAllRef.current ? { all: true } : undefined,
      );
      setData(result);
    } catch { /* ignore — stale data stays, matching useDashboard */ }
    setLoading(false);
  }, []);

  useEffect(() => {
    fetchDecisions();
    intervalRef.current = setInterval(fetchDecisions, 15_000);
    return () => clearInterval(intervalRef.current);
  }, [fetchDecisions]);

  // Event-driven refresh: react to decision_created / decision_resolved on the
  // daemon /events stream instead of waiting for the next 15s poll (#302).
  useEffect(() => {
    let ws: WebSocket | null = null;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let closed = false;
    const mountedAt = Date.now();

    const connect = () => {
      if (closed) return;
      const base = getApiBaseUrl().replace(/^http/, 'ws');
      try {
        ws = new WebSocket(`${base}/events`);
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
        const type = parsed.type ?? parsed.event_type ?? '';
        const ts = Date.parse(parsed.timestamp ?? '');
        if (Number.isFinite(ts) && ts < mountedAt) return; // skip replayed buffer
        if (type === 'decision_created' || type === 'decision_resolved') {
          fetchDecisions();
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
  }, [fetchDecisions]);

  /** Expand past the 10-item cap ("+M more"). Sticky for subsequent polls. */
  const showAll = useCallback(async () => {
    showAllRef.current = true;
    await fetchDecisions();
  }, [fetchDecisions]);

  /**
   * Answer one decision. 409 (already resolved elsewhere) is reported as
   * `{ ok: false, conflict: true }` so the item can show a refresh state;
   * other errors propagate.
   */
  const answer = useCallback(
    async (id: string, body: AnswerBody): Promise<AnswerResult> => {
      try {
        const outcome = await decisionsClient.answer(id, body);
        await fetchDecisions();
        return { ok: true, decision: outcome.decision, effect: outcome.effect };
      } catch (e) {
        if (e instanceof DecisionConflictError) {
          return { ok: false, conflict: true };
        }
        throw e;
      }
    },
    [fetchDecisions],
  );

  /** Resolved decisions + audit join for audit/history views. */
  const loadHistory = useCallback(async (): Promise<HistoryItem[]> => {
    const { items } = await decisionsClient.history();
    return items;
  }, []);

  return { data, loading, refresh: fetchDecisions, showAll, answer, loadHistory };
}
