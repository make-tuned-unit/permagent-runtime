/**
 * Decision Inbox — data hook (Lane L4).
 *
 * 15s polling, matching useDashboard's pattern. The daemon event stream
 * upgrade is filed as issue #302 (post-v1).
 */

import { useState, useEffect, useRef, useCallback } from 'react';
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
