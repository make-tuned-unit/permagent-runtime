// Goal-driven world activity — the honesty boundary for the Workshop bay.
//
// LAW (bible §4 sim-state + atmosphere honesty rule): every plaque and task
// light in the working bay represents a REAL active goal from
// /api/goals/active, refreshed on real `goal_state_changed` events plus a slow
// backstop poll. No fake work, no simulated tasks — a lit bench is a claim
// that the orchestrator is really doing something, so it must be true.
//
// Pattern mirrors atmosphere/worldSignals.ts: module store, updated only on
// network events (never per frame), plain getter for useFrame consumers and a
// useSyncExternalStore hook for React.

import { useSyncExternalStore } from 'react';
import { apiFetch } from '../../../lib/api';
import { subscribeWorldEvents } from '../shared/worldEvents';

export interface ActiveGoalLite {
  id: string;
  title: string;
  /** 'ready' | 'in_progress' (the ACTIVE_BINDINGS set). */
  state: string;
}

interface GoalActivityState {
  goals: ActiveGoalLite[];
  loaded: boolean;
}

/** The bay has 6 stools (WorkstationCluster) — the world shows at most 6 goals. */
export const BAY_CAPACITY = 6;

let state: GoalActivityState = { goals: [], loaded: false };
const subscribers = new Set<() => void>();

function publish(next: GoalActivityState): void {
  state = next;
  subscribers.forEach((fn) => fn());
}

async function refetch(): Promise<void> {
  try {
    const data = await apiFetch<{ goals: ActiveGoalLite[] }>('/api/goals/active');
    const goals = (data.goals ?? []).slice(0, BAY_CAPACITY);
    publish({ goals, loaded: true });
  } catch {
    // Daemon down: keep the last true state rather than flashing empty.
  }
}

let started = false;

export function ensureGoalActivity(): void {
  if (started) return;
  started = true;
  void refetch();
  // Live wire: the shared world socket (shared/worldEvents — one connection
  // for every world consumer). Refetch is idempotent, so replayed frames are
  // harmless triggers; the API stays the source of truth.
  subscribeWorldEvents((evt) => {
    if (evt.type === 'goal_state_changed') void refetch();
  });
  // Backstop poll — the socket is the live wire; this catches missed frames.
  setInterval(() => void refetch(), 60_000);
}

/** Plain snapshot for useFrame consumers (zero-alloc law: no copies). */
export function getActiveGoals(): ActiveGoalLite[] {
  return state.goals;
}

export function useActiveGoals(): GoalActivityState {
  ensureGoalActivity();
  return useSyncExternalStore(
    (fn) => {
      subscribers.add(fn);
      return () => subscribers.delete(fn);
    },
    () => state,
  );
}
