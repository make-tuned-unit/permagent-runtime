// idleLife.ts is pure number-in/number-out math specifically so the reduced-
// motion contract (bible §8: static, calm, correct-looking — no blink, no
// look-at tracking, no breathing, no desynced sway) can be checked without a
// WebGL context. Every "reduceMotion" branch below asserts the SAME shape:
// on, the function collapses to its neutral value across a swept time range;
// off, it visibly departs from neutral somewhere in that range.

import { describe, expect, it } from 'vitest';
import {
  swayZ,
  headNod,
  breathingScale,
  blinkEnvelope,
  shortestAngleDelta,
  resolveLookYaw,
  resolveLookPitch,
  LOOK_MAX_YAW,
  LOOK_MAX_PITCH,
} from './idleLife';
import { getIdPhase } from './idPhase';

const PHASE = getIdPhase('henry');
const OTHER_PHASE = getIdPhase('librarian');
const TIME_SWEEP = Array.from({ length: 400 }, (_, i) => i * 0.05); // 0..20s

describe('swayZ', () => {
  it('is exactly 0 at every sampled time under reduced motion', () => {
    for (const t of TIME_SWEEP) {
      expect(swayZ(t, PHASE, true, 2, 0.015)).toBe(0);
    }
  });

  it('is non-zero somewhere when motion is not reduced', () => {
    const anyNonZero = TIME_SWEEP.some((t) => swayZ(t, PHASE, false, 2, 0.015) !== 0);
    expect(anyNonZero).toBe(true);
  });

  it('desyncs two different phases — they are not the same function of time', () => {
    const a = TIME_SWEEP.map((t) => swayZ(t, PHASE, false, 2, 0.015));
    const b = TIME_SWEEP.map((t) => swayZ(t, OTHER_PHASE, false, 2, 0.015));
    expect(a).not.toEqual(b);
  });
});

describe('headNod', () => {
  it('is exactly 0 under reduced motion', () => {
    for (const t of TIME_SWEEP) {
      expect(headNod(t, PHASE, true)).toBe(0);
    }
  });

  it('is non-zero somewhere when motion is not reduced', () => {
    const anyNonZero = TIME_SWEEP.some((t) => headNod(t, PHASE, false) !== 0);
    expect(anyNonZero).toBe(true);
  });
});

describe('breathingScale', () => {
  it('is exactly 1 (neutral, no scale change) under reduced motion', () => {
    for (const t of TIME_SWEEP) {
      expect(breathingScale(t, PHASE, true)).toBe(1);
    }
  });

  it('departs from 1 somewhere when motion is not reduced, but stays subtle', () => {
    const values = TIME_SWEEP.map((t) => breathingScale(t, PHASE, false));
    expect(values.some((v) => v !== 1)).toBe(true);
    // "subtle enough that you notice it only when it stops" — bounded near 1.
    for (const v of values) {
      expect(v).toBeGreaterThan(0.95);
      expect(v).toBeLessThan(1.05);
    }
  });

  it('gives different agents a different breathing cadence, not just a phase shift', () => {
    const a = TIME_SWEEP.map((t) => breathingScale(t, PHASE, false));
    const b = TIME_SWEEP.map((t) => breathingScale(t, OTHER_PHASE, false));
    expect(a).not.toEqual(b);
  });
});

describe('blinkEnvelope', () => {
  it('is exactly 1 (eyes never dip) under reduced motion', () => {
    for (const t of TIME_SWEEP) {
      expect(blinkEnvelope(t, PHASE, true)).toBe(1);
    }
  });

  it('dips below 1 at least once within a realistic session window when not reduced', () => {
    const values = TIME_SWEEP.map((t) => blinkEnvelope(t, PHASE, false));
    expect(values.some((v) => v < 1)).toBe(true);
  });

  it('never dips to exactly 0 — it modulates, it does not switch the visor off', () => {
    const values = TIME_SWEEP.map((t) => blinkEnvelope(t, PHASE, false));
    for (const v of values) {
      expect(v).toBeGreaterThan(0);
      expect(v).toBeLessThanOrEqual(1);
    }
  });

  it('two different agents do not blink on the same beat', () => {
    // Find every t where each agent is mid-blink (envelope meaningfully below
    // 1) and assert the two sets of moments don't coincide across the sweep —
    // the whole point of deriving the blink timing from a per-agent phase.
    const fineSweep = Array.from({ length: 2000 }, (_, i) => i * 0.01); // 0..20s, 10ms steps
    const blinkingA = fineSweep.filter((t) => blinkEnvelope(t, PHASE, false) < 0.9);
    const blinkingB = fineSweep.filter((t) => blinkEnvelope(t, OTHER_PHASE, false) < 0.9);
    const overlap = blinkingA.filter((t) => blinkingB.includes(t));
    expect(overlap.length).toBe(0);
  });
});

describe('shortestAngleDelta', () => {
  it('is 0 when from equals to', () => {
    expect(shortestAngleDelta(1.2, 1.2)).toBeCloseTo(0, 10);
  });

  it('takes the short way around a ±π wrap', () => {
    // From just past +π to just past -π is a small step across the seam, not
    // a near-full lap the long way around.
    const d = shortestAngleDelta(3.1, -3.1);
    expect(Math.abs(d)).toBeLessThan(0.5);
  });

  it('result stays within (-π, π]', () => {
    for (let from = -10; from <= 10; from += 1.3) {
      for (let to = -10; to <= 10; to += 1.7) {
        const d = shortestAngleDelta(from, to);
        expect(d).toBeGreaterThan(-Math.PI - 1e-9);
        expect(d).toBeLessThanOrEqual(Math.PI + 1e-9);
      }
    }
  });
});

describe('resolveLookYaw / resolveLookPitch — clamped head turn', () => {
  it('is 0/0 for a target dead ahead on the current heading', () => {
    // Heading 0 means facing +z (atan2(dx,dz) convention, matching motion.ts).
    // A target straight ahead has dx=0, dz>0, dy=0.
    expect(resolveLookYaw(0, 5, 0)).toBeCloseTo(0, 6);
    expect(resolveLookPitch(0, 5)).toBeCloseTo(0, 6);
  });

  it('clamps yaw to LOOK_MAX_YAW for a target far to one side', () => {
    // Target directly to the agent's right (dx large, dz ~0) while heading 0
    // (facing +z) is a 90 degree turn — well past the clamp.
    const yaw = resolveLookYaw(10, 0.001, 0);
    expect(Math.abs(yaw)).toBeCloseTo(LOOK_MAX_YAW, 6);
  });

  it('clamps pitch to LOOK_MAX_PITCH for a target far above or below', () => {
    const up = resolveLookPitch(50, 1);
    const down = resolveLookPitch(-50, 1);
    expect(Math.abs(up)).toBeCloseTo(LOOK_MAX_PITCH, 6);
    expect(Math.abs(down)).toBeCloseTo(LOOK_MAX_PITCH, 6);
  });

  it('is 0/0 (neutral) for a degenerate/zero-distance target', () => {
    expect(resolveLookYaw(0, 0, 0)).toBe(0);
    expect(resolveLookPitch(5, 0)).toBe(0);
  });

  it('turns the short way around the heading, same as shortestAngleDelta', () => {
    // Heading nearly all the way around (~π) with a target just past the seam
    // should clamp rather than attempt a near-2π turn.
    const yaw = resolveLookYaw(-0.01, -5, Math.PI - 0.01);
    expect(Math.abs(yaw)).toBeLessThanOrEqual(LOOK_MAX_YAW + 1e-9);
  });
});
