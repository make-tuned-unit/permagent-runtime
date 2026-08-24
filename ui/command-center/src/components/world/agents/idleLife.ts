// Per-agent idle "life" — WORLD_VIEW_BIBLE.md §4 body language, §8 perf/reduced-
// motion law. This is the pure-math half of the desync work: given a shared
// clock reading, a per-agent phase (idPhase.ts), and the reduced-motion flag,
// every function here returns a plain number describing one small continuous
// flourish (sway angle, breathing scale, a nod offset, a blink's intensity
// multiplier, a clamped head-turn angle). Nothing here touches THREE, refs, or
// materials — AgentCharacterV2 is the only caller, and it applies these
// numbers to its pre-allocated bones/materials inside `useFrame`. Keeping the
// math pure means the reduced-motion contract (§8: "static fallback ... for
// particles, arcs, tour, reactive ambience") can be checked with plain
// `expect()` calls, no WebGL context required.
//
// The shape of every function is deliberately `(time, phase, reduceMotion,
// ...) → number`: reduceMotion is always the thing that collapses the
// function to its neutral value (0 offset, 1× scale, no dip), never a branch
// that changes WHAT is being computed.

/**
 * Desynced ambient sway (root Z-tilt) — the same slow weight-shift the scene
 * already had, phase-shifted per agent so seven agents don't rock together.
 * One function serves both the idle sway and the tending haul-sway in
 * AgentCharacterV2 (they differ only in `freq`/`amplitude`, not in shape).
 */
export function swayZ(
  t: number,
  phase: number,
  reduceMotion: boolean,
  freq: number,
  amplitude: number,
): number {
  if (reduceMotion) return 0;
  return Math.sin(t * freq + phase) * amplitude;
}

/**
 * Desynced periodic head nod (engaged poses only) — a product of a fast nod
 * frequency and a slow "am I nodding right now" envelope, both phase-shifted
 * together so the whole gesture — not just one of its two sines — moves with
 * the agent's own rhythm.
 */
export function headNod(t: number, phase: number, reduceMotion: boolean): number {
  if (reduceMotion) return 0;
  return 0.05 * Math.sin(t * 2.6 + phase) * Math.max(0, Math.sin(t * 0.45 + phase));
}

/** Tending's arm haul sway — the "unhurried stoop-and-place" cadence (poses.ts
 * TENDING pose), phase-shifted per agent. */
export function tendingHaul(t: number, phase: number, reduceMotion: boolean): number {
  if (reduceMotion) return 0;
  return Math.sin(t * 1.1 + phase) * 0.18;
}

/** Tending's spine lean — the forward half of the same haul cycle (rectified,
 * so the lean only ever pitches forward, never backward past neutral). */
export function tendingSpineLean(t: number, phase: number, reduceMotion: boolean): number {
  if (reduceMotion) return 0;
  return Math.max(0, Math.sin(t * 1.1 + phase)) * 0.06;
}

const BREATH_BASE_RATE = 1.4; // rad/s — a resting breathing cadence
const BREATH_RATE_SPREAD = 0.35; // per-agent variation band
const BREATH_AMPLITUDE = 0.012; // ±1.2% torso scale — noticeable only if it stopped

/**
 * Subtle torso breathing (bible §4 "subtle breathing... a small vertical/scale
 * oscillation on the torso"). Returns a scale multiplier to apply to the spine
 * bone (the torso proxy in this rig — there's no separate chest bone). Both
 * the rate and the phase are per-agent, so this isn't just phase-shifted, it's
 * a genuinely different cadence per agent — reads as organic rather than one
 * animation played back at seven different start times.
 */
export function breathingScale(t: number, phase: number, reduceMotion: boolean): number {
  if (reduceMotion) return 1;
  const rate = BREATH_BASE_RATE + ((phase * 0.6) % 1) * BREATH_RATE_SPREAD;
  return 1 + Math.sin(t * rate + phase) * BREATH_AMPLITUDE;
}

const BLINK_BASE_S = 5; // center of the bible's 3–7s range
const BLINK_SPREAD_S = 2; // half-width — period wanders between 3s and 7s
const BLINK_DIP_S = 0.15; // ~150ms dip-and-recover (bible §4: "roughly 120-180ms")
const BLINK_DEPTH = 0.85; // multiplier dips to (1 - BLINK_DEPTH) at the trough, never to 0

function mod(a: number, n: number): number {
  return ((a % n) + n) % n;
}

/**
 * Blink envelope — 1 outside a blink, dipping toward a low (not zero, an
 * eyelid still passes some light through fog/emissive glow) trough for about
 * 150ms during one. The "randomized 3-7s interval with per-agent jitter" the
 * bible asks for comes from beating a slow per-agent sine against the base
 * period instead of a fixed metronome or a stored counter: the effective
 * period drifts between ~3s and ~7s over tens of seconds, and the drift rate
 * and starting phase are both derived from `phase`, so no two agents ever
 * blink on the same beat. Closed-form and O(1) per call — no loop, nothing
 * accumulated across frames — so the cost never grows with session length.
 *
 * The caller MULTIPLIES this onto whatever visor emissiveIntensity the state
 * channel already computed (bible §4: "modulates intensity around whatever
 * the current state intensity is, never sets an absolute value, never changes
 * hue") — this function never returns a color and never claims to know the
 * base intensity.
 */
export function blinkEnvelope(t: number, phase: number, reduceMotion: boolean): number {
  if (reduceMotion) return 1;
  const blinkPhase = phase * 1.7 + 0.9; // decorrelated from the sway/breathing phase
  const wobbleHz = 0.015 + ((phase * 0.31) % 1) * 0.01; // very slow per-agent drift
  const period = BLINK_BASE_S + BLINK_SPREAD_S * Math.sin(t * wobbleHz + blinkPhase);
  const cycle = mod(t + blinkPhase * period, period);
  if (cycle > BLINK_DIP_S) return 1;
  const u = cycle / BLINK_DIP_S; // 0..1 across the dip window
  const envelope = u < 0.5 ? u * 2 : (1 - u) * 2; // 0 -> 1 -> 0 triangular, peak at mid-blink
  return 1 - envelope * BLINK_DEPTH;
}

/** A person turns their head roughly this far before their body follows (bible §4). */
export const LOOK_MAX_YAW = (60 * Math.PI) / 180;
/** Pitch reads as unnatural sooner than yaw does — keep it tighter. */
export const LOOK_MAX_PITCH = (30 * Math.PI) / 180;

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/**
 * Shortest signed angular delta from `from` to `to` (both radians), result in
 * (-π, π]. Turning a head/body the short way around a wrap (e.g. from 3.1 to
 * -3.1 radians, which are 0.08 rad apart, not ~6.2 rad apart) is what keeps a
 * look-at from ever visibly spinning the long way round.
 */
export function shortestAngleDelta(from: number, to: number): number {
  let d = (to - from) % (Math.PI * 2);
  if (d > Math.PI) d -= Math.PI * 2;
  else if (d < -Math.PI) d += Math.PI * 2;
  return d;
}

/**
 * Desired head yaw (local, additive on top of the body's own facing/heading),
 * clamped to LOOK_MAX_YAW. `dx`/`dz` are target-minus-agent in world space;
 * `heading` is the agent's current body yaw (same atan2(dx,dz) convention
 * motion.ts uses for locomotion facing, so the two stay consistent).
 */
export function resolveLookYaw(dx: number, dz: number, heading: number): number {
  if (Math.abs(dx) < 1e-4 && Math.abs(dz) < 1e-4) return 0;
  const worldYaw = Math.atan2(dx, dz);
  return clamp(shortestAngleDelta(heading, worldYaw), -LOOK_MAX_YAW, LOOK_MAX_YAW);
}

/**
 * Desired head pitch (local X-rotation in this rig's convention: positive
 * looks down, negative looks up — matches poses.ts, e.g. the error slump's
 * `head: [0.35, 0, 0]`), clamped to LOOK_MAX_PITCH. `dy` is target-minus-head
 * height; `flatDist` is the horizontal distance to the target.
 */
export function resolveLookPitch(dy: number, flatDist: number): number {
  if (flatDist < 1e-4) return 0;
  const elevation = Math.atan2(dy, flatDist); // positive when the target is above head height
  return clamp(-elevation, -LOOK_MAX_PITCH, LOOK_MAX_PITCH);
}
