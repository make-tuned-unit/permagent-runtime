// The Watcher's nudge — a REAL proactive_nudge event made physical.
//
// The Watcher (Echo, #672) is the daemon's proactive worker: at most once a
// day it surfaces the single most useful thing — a dormant Brain thread or
// fresh project news — as a `proactive_nudge` on /events. It has no live
// status endpoint yet (documented gap), so in the world it lives as ambient
// sim life (idle/available under the §4 clamp — it can never fake work). But
// the nudge itself IS a real event, and when one arrives live:
//
//   • the vigil beacon on its tower flares once (a 4s bloom),
//   • the Watcher walks to wherever Henry stands and presents (behavior.ts),
//   • a plaque shows the REAL nudge subject for ~75s, then fades.
//
// Replayed buffer events are ignored (the world only animates work it saw
// happen). No nudge ⇒ dark beacon, empty plaque, the Watcher just keeps watch.
//
// PURE CORE (unit-tested): noteNudge/resolveNudgeDisplay — a monotonic seq +
// timestamps in, deterministic flare/plaque phases out. Zero per-frame
// allocation: useFrame consumers read getNudge() + resolveNudgeDisplay with
// the frame clock.

import { useSyncExternalStore } from 'react';

export interface NudgeState {
  /** Monotonic sequence — 0 means "no nudge this session". */
  seq: number;
  kind: string;
  subject: string;
  message: string;
  count: number;
  /** Epoch ms the nudge arrived (live). */
  at: number;
}

export interface NudgeDisplay {
  /** Whether any nudge presentation is in progress. */
  active: boolean;
  /** Beacon flare strength [0,1] — a single 4s bell curve on arrival. */
  flare: number;
  /** Plaque opacity [0,1] — holds, then fades over the final 10s. */
  plaqueAlpha: number;
}

export const NUDGE_FLARE_MS = 4_000;
export const NUDGE_PRESENT_MS = 75_000;
const PLAQUE_FADE_MS = 10_000;

const state: NudgeState = { seq: 0, kind: '', subject: '', message: '', count: 0, at: 0 };
const listeners = new Set<() => void>();
let snapshot: NudgeState = { ...state };

/** Record a REAL live proactive_nudge (called from the /events binding). */
export function noteNudge(
  payload: { kind?: unknown; subject?: unknown; message?: unknown; count?: unknown },
  now: number,
): void {
  state.seq += 1;
  state.kind = typeof payload.kind === 'string' ? payload.kind : '';
  state.subject = typeof payload.subject === 'string' ? payload.subject : '';
  state.message = typeof payload.message === 'string' ? payload.message : '';
  state.count = typeof payload.count === 'number' ? payload.count : 0;
  state.at = now;
  snapshot = { ...state };
  listeners.forEach((l) => l());
}

/** Zero-alloc getter for useFrame loops. */
export function getNudge(): NudgeState {
  return state;
}

/** React hook — re-renders only when a new nudge arrives. */
export function useNudge(): NudgeState {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => snapshot,
  );
}

/**
 * Pure phase resolution (unit-tested). now/at in the same clock (epoch ms).
 *   flare: sin bell over the first NUDGE_FLARE_MS.
 *   plaqueAlpha: 1 while presenting, linear fade over the last PLAQUE_FADE_MS.
 */
export function resolveNudgeDisplay(n: Pick<NudgeState, 'seq' | 'at'>, now: number): NudgeDisplay {
  if (n.seq <= 0) return { active: false, flare: 0, plaqueAlpha: 0 };
  const age = now - n.at;
  if (age < 0 || age >= NUDGE_PRESENT_MS) return { active: false, flare: 0, plaqueAlpha: 0 };
  const flare = age < NUDGE_FLARE_MS ? Math.sin((age / NUDGE_FLARE_MS) * Math.PI) : 0;
  const fadeStart = NUDGE_PRESENT_MS - PLAQUE_FADE_MS;
  const plaqueAlpha = age <= fadeStart ? 1 : 1 - (age - fadeStart) / PLAQUE_FADE_MS;
  return { active: true, flare, plaqueAlpha: Math.max(0, Math.min(1, plaqueAlpha)) };
}
