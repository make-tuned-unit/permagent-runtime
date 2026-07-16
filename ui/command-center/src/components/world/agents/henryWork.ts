// Henry's live work detail — the REAL in-flight tool name.
//
// /api/henry/status has carried `current_tool` since #84 (the daemon reads the
// active session's in-flight tool call), but the world never showed it. This
// store is fed by the SAME 2s poll stateSources already runs for Henry's HUD
// state (one endpoint, two truths — no extra network), and the HenryToolSigil
// renders it as a small holo chip over his head while he works.
//
// HONESTY: the chip exists only while the poll reports a tool. No tool ⇒ no
// chip; poll failure clears it (an unreachable daemon must not leave a stale
// claim floating over Henry).

import { useSyncExternalStore } from 'react';

export interface HenryWork {
  /** Real in-flight tool name (e.g. "web_search"), or null when none. */
  tool: string | null;
  /** Real count of daemon tasks in flight. */
  tasksInFlight: number;
}

/**
 * Pure extraction from the /api/henry/status response shape (unit-tested):
 * empty/whitespace tool strings normalize to null, counts to non-negative ints.
 */
export function extractHenryWork(resp: {
  current_tool?: string | null;
  tasks_in_flight?: number;
}): HenryWork {
  const raw = typeof resp.current_tool === 'string' ? resp.current_tool.trim() : '';
  const tasks = typeof resp.tasks_in_flight === 'number' ? resp.tasks_in_flight : 0;
  return {
    tool: raw.length > 0 ? raw : null,
    tasksInFlight: Number.isFinite(tasks) && tasks > 0 ? Math.floor(tasks) : 0,
  };
}

let work: HenryWork = { tool: null, tasksInFlight: 0 };
const listeners = new Set<() => void>();

export function setHenryWork(next: HenryWork): void {
  if (next.tool === work.tool && next.tasksInFlight === work.tasksInFlight) return;
  work = next;
  listeners.forEach((l) => l());
}

export function getHenryWork(): HenryWork {
  return work;
}

export function useHenryWork(): HenryWork {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => work,
  );
}
