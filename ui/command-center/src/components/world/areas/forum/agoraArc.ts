// Agora interaction arc — the reversible body⇄code state machine (#306 prep).
//
// THE ARC (one system played forward on enter, backward on exit):
//   home     → embodied: the sovereign agent stands in the warm marble Rotunda.
//   entering → DISSOLVE: at the portal the embodied rig unravels into a stream of
//              glowing code (chiti-ID colour) that flows through the membrane.
//   witness  → ABSORBED: the stream coalesces into the agent's glyph; the swarm
//              receives it (a ripple of acknowledgment) and the user holds in the
//              Agora watching the living constellation breathe.
//   exiting  → REMATERIALIZE: the mirror image — code streams back through the
//              portal and reforms the embodied rig on the Rotunda side. Home.
//
// `dissolve` is the single continuous scalar: 0 = fully embodied (home), 1 = fully
// code (witness). entering ramps it 0→1, exiting ramps 1→0. Every visual beat
// (portal stream, rig fade, glyph ignition, absorb ripple, camera dive) reads this
// one number, so forward and reverse are guaranteed to mirror.
//
// LAW: `dissolve` changes every frame and is read from useFrame via a plain
// getter (zero React work/allocation per frame). Only the discrete PHASE is
// published to React subscribers (drives the exit affordance + Henry's mount).

import { useSyncExternalStore } from 'react';

export type AgoraPhase = 'home' | 'entering' | 'witness' | 'exiting';

/** World-space center of the Agora volume, beyond the Stargate on the NW
 *  diagonal (local +x=28 of the forum group → world via zoneCenter math). The
 *  MouthShaft portal daylight hangs directly above this point. */
export const AGORA_CENTER: [number, number, number] = [-19.8, 3, -19.8];
/** Where the camera returns to on exit — framing the warm Rotunda crown. */
export const HALL_HOME: [number, number, number] = [0, 2, 2];

const ENTER_DUR = 1.35; // seconds (ruling: ~1-1.5s, eased)
const EXIT_DUR = 1.25;

interface ArcState {
  phase: AgoraPhase;
  /** 0 = embodied (home) … 1 = pure code (witness). */
  dissolve: number;
}

const state: ArcState = { phase: 'home', dissolve: 0 };
const listeners = new Set<() => void>();

function publish(): void {
  listeners.forEach((l) => l());
}

/** Cubic ease for the eased dissolve read (progress feel, not linear). */
export function easeArc(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

/** useFrame accessor — no allocation, no React work. */
export function getArc(): Readonly<ArcState> {
  return state;
}
/** Eased embodiment→code scalar for the visual beats. */
export function getDissolve(): number {
  return easeArc(state.dissolve);
}

export function enterAgora(): void {
  if (state.phase === 'home' || state.phase === 'exiting') {
    state.phase = 'entering';
    publish();
  }
}

export function exitAgora(): void {
  if (state.phase === 'witness' || state.phase === 'entering') {
    state.phase = 'exiting';
    publish();
  }
}

/** Advance the arc. Called once per frame by the Agora ticker. Publishes only on
 *  a discrete phase change (entering→witness, exiting→home), never per frame. */
export function tickArc(dt: number): void {
  if (state.phase === 'entering') {
    state.dissolve = Math.min(1, state.dissolve + dt / ENTER_DUR);
    if (state.dissolve >= 1) {
      state.phase = 'witness';
      publish();
    }
  } else if (state.phase === 'exiting') {
    state.dissolve = Math.max(0, state.dissolve - dt / EXIT_DUR);
    if (state.dissolve <= 0) {
      state.phase = 'home';
      publish();
    }
  }
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

/** React hook — re-renders only on discrete phase changes. */
export function useAgoraPhase(): AgoraPhase {
  return useSyncExternalStore(subscribe, () => state.phase);
}
