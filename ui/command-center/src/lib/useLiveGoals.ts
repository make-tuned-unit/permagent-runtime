/**
 * Live active-goal state — the single source of truth for every dashboard
 * "in flight" surface (the count, the list, the "N active" header, and the
 * "Henry is working on N things" status). Because count and list both come from
 * one `/api/goals/active` query, they can never disagree (the bug this fixes).
 *
 * Subscribes via the shared {@link useGoalEvents} primitive so it updates within
 * a frame of any goal transition, with a 15s poll as a reconnect-gap backstop
 * (matching useDecisions). Call this ONCE in the dashboard parent and pass the
 * result down to the cards — do not call it per card.
 *
 * "Active" excludes Triage (queued, not started), Review (waiting on the user —
 * surfaced in the Decision Inbox), Complete (done), parked, and archived goals
 * — enforced server-side by GoalState::ACTIVE_BINDINGS.
 */

import { useCallback, useEffect, useState } from 'react';
import { getApiBaseUrl, loadDaemonToken } from './api';
import { useGoalEvents } from './useGoalEvents';

export interface ActiveGoal {
  id: string;
  title: string;
  project_id: string;
  /** ready | in_progress */
  state: string;
  assigned_to: string | null;
  created_at: string;
  updated_at: string;
}

interface ActiveGoalsResponse {
  count: number;
  goals: ActiveGoal[];
}

/**
 * True when two goal lists are field-for-field equal. Lets us keep the previous
 * array (and its element identities) when a refetch returns identical data, so
 * the "in flight" cards don't re-render — and the animated Mobius logos don't
 * flicker — on every poll tick or goal event that didn't actually change them.
 */
function sameGoals(a: ActiveGoal[], b: ActiveGoal[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((g, i) => {
    const o = b[i];
    return g.id === o.id && g.state === o.state && g.title === o.title
      && g.assigned_to === o.assigned_to && g.updated_at === o.updated_at;
  });
}

export function useLiveGoals() {
  const [goals, setGoals] = useState<ActiveGoal[]>([]);
  const [loaded, setLoaded] = useState(false);

  const fetchGoals = useCallback(async () => {
    try {
      const token = await loadDaemonToken();
      const res = await fetch(`${getApiBaseUrl()}/api/goals/active`, {
        headers: token ? { Authorization: `Bearer ${token}` } : undefined,
      });
      if (!res.ok) return;
      const data: ActiveGoalsResponse = await res.json();
      const next = data.goals ?? [];
      setGoals(prev => (sameGoals(prev, next) ? prev : next));
    } catch {
      /* ignore — stale data stays, matching useDashboard */
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    fetchGoals();
    const id = setInterval(fetchGoals, 15_000);
    return () => clearInterval(id);
  }, [fetchGoals]);

  useGoalEvents(fetchGoals);

  return { goals, activeCount: goals.length, loaded, refresh: fetchGoals };
}
