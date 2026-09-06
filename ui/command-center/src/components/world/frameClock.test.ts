/**
 * FrameCap's driven clock — the monotonic-accumulation invariant under test:
 * given any sequence of now-timestamps, including a multi-minute gap that
 * models a hide/show tab round-trip, the deltas the clock implies must never
 * be negative and never exceed MAX_FRAME_STEP_S. Violating either is exactly
 * what let THREE.MathUtils.damp's exp(lambda * dt) overflow to Infinity/NaN
 * (the World white-out bug).
 */

import { describe, expect, it } from 'vitest';
import { createFrameClock, stepFrameClock, MAX_FRAME_STEP_S, worldFrameDue, WORLD_TARGET_FPS } from './frameClock';

/** Old (buggy) baseline logic, kept ONLY to demonstrate the regression this
 * file guards against — this is what FrameCap did before the fix: capture
 * `t0` once per effect run and derive elapsed as `(now - t0) / 1000`. Because
 * the effect re-runs on every `active` re-activation, t0 resets to a fresh
 * baseline while the previously-reported elapsed value stays wherever it was
 * — producing an implied delta that can go arbitrarily negative. */
function naiveReactivatedElapsed(nowsSinceReactivation: number[]): number[] {
  const t0 = nowsSinceReactivation[0];
  return nowsSinceReactivation.map((now) => (now - t0) / 1000);
}

function deltas(values: number[]): number[] {
  const out: number[] = [];
  for (let i = 1; i < values.length; i++) out.push(values[i] - values[i - 1]);
  return out;
}

describe('frameClock', () => {
  it('allows every 60 Hz frame without wasting 120 Hz renders', () => {
    expect(WORLD_TARGET_FPS).toBe(60);
    expect(worldFrameDue(16.5, 0)).toBe(true);
    expect(worldFrameDue(8.33, 0)).toBe(false);
    expect(worldFrameDue(33.3, 0)).toBe(true);
    expect(worldFrameDue(0, -Infinity)).toBe(true);
  });
  it('starts at 0 and does not advance on the very first step', () => {
    const clock = createFrameClock();
    expect(stepFrameClock(clock, 1000)).toBe(0);
  });

  it('accumulates real elapsed time under steady ~30fps ticks', () => {
    const clock = createFrameClock();
    const nows = [0, 33, 66, 100, 133];
    const elapsed = nows.map((n) => stepFrameClock(clock, n));
    // Monotonic, and tracks wall time closely at a normal frame cadence.
    for (let i = 1; i < elapsed.length; i++) {
      expect(elapsed[i]).toBeGreaterThan(elapsed[i - 1]);
    }
    expect(elapsed[elapsed.length - 1]).toBeCloseTo(0.133, 5);
  });

  it('clamps a re-activation gap instead of jumping or going negative', () => {
    const clock = createFrameClock();
    // Three minutes "in the World" at a steady cadence...
    const activeNows = [0, 33, 66, 100, 133];
    activeNows.forEach((n) => stepFrameClock(clock, n));
    const beforeGap = clock.elapsed;

    // ...tab switch away, three minutes pass with the loop stopped (no
    // step() calls at all — this is the gap itself)...
    const gapNow = 133 + 3 * 60 * 1000;

    // ...then the tab comes back and the loop resumes.
    const afterGap = stepFrameClock(clock, gapNow);

    expect(afterGap).toBeGreaterThanOrEqual(beforeGap); // never negative
    expect(afterGap - beforeGap).toBeLessThanOrEqual(MAX_FRAME_STEP_S); // never huge
  });

  it('property: no step across any sequence — including a reactivation gap — is negative or exceeds MAX_FRAME_STEP_S', () => {
    const clock = createFrameClock();
    const sequence = [
      0, 33, 66, 100, 133, 166, // active for a while
      // gap: hidden for ~3.5 minutes
      166 + 210_000,
      // active again
      166 + 210_000 + 33,
      166 + 210_000 + 66,
      // gap: hidden again, much longer (~11 min, the diagnosis's T≈177s+ case
      // scaled up)
      166 + 210_000 + 66 + 660_000,
      166 + 210_000 + 66 + 660_000 + 33,
    ];
    const elapsed = sequence.map((n) => stepFrameClock(clock, n));
    const steps = deltas(elapsed);
    for (const step of steps) {
      expect(step).toBeGreaterThanOrEqual(0);
      expect(step).toBeLessThanOrEqual(MAX_FRAME_STEP_S);
    }
    // And the value handed downstream is monotonic throughout.
    for (let i = 1; i < elapsed.length; i++) {
      expect(elapsed[i]).toBeGreaterThanOrEqual(elapsed[i - 1]);
    }
  });

  it('regression: the pre-fix baseline-per-activation logic produces a deeply negative implied delta; the fixed helper does not', () => {
    // Model: 177s spent in the World (matches the diagnosis's fog.density
    // Infinity threshold at T≈177s for lambda=4), then a tab round-trip.
    const T_MS = 177_000;

    // OLD LOGIC: reproduce what FrameCap's effect used to do. First
    // "activation" runs t0 = 0 and reports elapsed up to T_MS/1000. Then the
    // effect is torn down and re-run (tab switch back) — a NEW t0 baseline is
    // captured from performance.now() at that moment, which in real wall-clock
    // terms is far past T_MS. The very next accepted tick after that new
    // baseline reports elapsed ~= 0 again.
    const firstActivation = naiveReactivatedElapsed([0, T_MS]);
    const lastElapsedBeforeGap = firstActivation[firstActivation.length - 1];
    // Re-activation: fresh t0, first tick right at reactivation.
    const secondActivation = naiveReactivatedElapsed([T_MS + 5, T_MS + 5 + 33]);
    const impliedDeltaOld = secondActivation[0] - lastElapsedBeforeGap;
    expect(impliedDeltaOld).toBeLessThan(-100); // reproduces the bug: deeply negative

    // NEW LOGIC: the same wall-clock sequence through the fixed helper never
    // goes backward and never exceeds one clamped step.
    const clock = createFrameClock();
    stepFrameClock(clock, 0);
    const beforeGapFixed = stepFrameClock(clock, T_MS);
    const afterGapFixed = stepFrameClock(clock, T_MS + 5);
    expect(afterGapFixed).toBeGreaterThanOrEqual(beforeGapFixed);
    expect(afterGapFixed - beforeGapFixed).toBeLessThanOrEqual(MAX_FRAME_STEP_S);
  });
});
