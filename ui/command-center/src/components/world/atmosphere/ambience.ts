// W4 atmosphere — activity-reactive ambience level (bible §7).
// The room responds to how many agents are actually WORKING: column-vein
// emissive intensity, dust rate, and orbital-arc speed all scale gently from
// this single smoothed scalar. LAW: ≤ 1.5× idle values, smooth ramps, zero
// per-frame allocations. Driven only by shared/agentStatus (the honesty
// boundary — sim agents are clamped there and can never claim 'working').

import { getAgentRuntimeStates } from '../shared/agentStatus';

/** Idle level. Everything multiplies by the current level, so 1 = no change. */
const IDLE_LEVEL = 1;
/** LAW (bible §7): reactive ambience never exceeds 1.5× idle values. */
const MAX_LEVEL = 1.5;
/** Working agents needed to reach full busy-room ambience. */
const FULL_BUSY_COUNT = 3;
/** Ramp speed — exponential approach, ~1.5s to cover 90% of a step. */
const RAMP_RATE = 1.5;

let level = IDLE_LEVEL;
let frozen = false;

/**
 * reduceMotion static fallback: lock the level at idle so reactive ambience
 * stops animating (bible §8 reduceMotion law). Set once by AmbienceTicker.
 */
export function setAmbienceFrozen(value: boolean): void {
  frozen = value;
  if (frozen) level = IDLE_LEVEL;
}

/**
 * Advance the smoothed level toward the target derived from the live working
 * count. Called exactly once per frame by AmbienceTicker (priority -100, so
 * every consumer in the same frame sees a consistent value). No allocations.
 */
export function tickAmbience(delta: number): void {
  if (frozen) return;
  const states = getAgentRuntimeStates();
  let working = 0;
  for (let i = 0; i < states.length; i++) {
    if (states[i].hudState === 'working') working++;
  }
  const target =
    IDLE_LEVEL +
    (MAX_LEVEL - IDLE_LEVEL) * Math.min(working / FULL_BUSY_COUNT, 1);
  // Exponential smoothing — frame-rate independent, no overshoot.
  level += (target - level) * Math.min(1, delta * RAMP_RATE);
}

/** Current ambience level in [1, 1.5]. Safe to call from any useFrame loop. */
export function getAmbienceLevel(): number {
  return level;
}
