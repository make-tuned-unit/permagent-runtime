// FrameCap's driven clock (WorldView.tsx), extracted so the monotonic-
// accumulation logic can be unit tested without a Canvas.
//
// THE BUG this exists to prevent: R3F with `frameloop="never"` computes
// `delta = timestamp - clock.elapsedTime` with no clamp. FrameCap used to
// derive its timestamp from a fresh `performance.now()` baseline captured
// inside the effect (`const t0 = performance.now()`), and that effect re-runs
// every time `active` flips — which happens on every workspace tab
// round-trip (useWorldVisibility → ResizeObserver → display:none). After a
// multi-minute stay in the World, switching tabs and back re-zeroed `t0`
// while r3f's internal clock.elapsedTime was still sitting at the old large
// value, so the first `advance()` after return passed a timestamp far BEHIND
// where the clock already was — a deeply negative delta. THREE.MathUtils.damp
// computes `exp(lambda * dt)`; a large negative dt overflows that to
// Infinity, then NaN, and NaN fog/light values poison the whole frame (the
// reported white-out with color fringing).
//
// The fix: never re-derive the timeline from a fresh baseline. Accumulate
// elapsed seconds in a value that survives across effect re-runs (a ref, at
// the call site), stepping forward by real wall-clock time each frame but
// capping any single step — including the step across a re-activation gap —
// so the accumulated value can never move backward and never jump by more
// than one frame's worth of simulated time.

/** Cap on a single frame step, in seconds. Also the max delta any downstream
 * consumer (Atmosphere dampers, agent halos, etc.) will ever see from this
 * clock. Chosen well above one frame at the 30fps target (~33ms) so normal
 * playback is never visibly clamped, and well below anything that would let
 * THREE.MathUtils.damp's exp(lambda * dt) misbehave. */
export const MAX_FRAME_STEP_S = 0.1;

export interface FrameClock {
  /** Total elapsed seconds to hand to r3f's advance(). Monotonic — only ever
   * increases, no matter how long a gap between step() calls was. */
  elapsed: number;
  /** performance.now() timestamp of the previous step() call, or null before
   * the first one (or after a reset). */
  lastNow: number | null;
}

export function createFrameClock(): FrameClock {
  return { elapsed: 0, lastNow: null };
}

/**
 * Advance `clock` to `now` (a performance.now()-style timestamp) and return
 * the new elapsed value to pass to advance(). Safe to call across an
 * arbitrarily long gap since the previous call (e.g. the World tab was
 * hidden for minutes): the contributed step is clamped to
 * [0, MAX_FRAME_STEP_S], so the returned value can never decrease and can
 * never jump forward by more than one frame's worth of simulated time.
 */
export function stepFrameClock(clock: FrameClock, now: number): number {
  if (clock.lastNow !== null) {
    const stepS = (now - clock.lastNow) / 1000;
    clock.elapsed += Math.min(Math.max(stepS, 0), MAX_FRAME_STEP_S);
  }
  clock.lastNow = now;
  return clock.elapsed;
}
