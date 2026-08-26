import { describe, it, expect } from 'vitest';
import {
  ORB_FLOOR_CEILING,
  orbAmp,
  orbBands,
  orbMotionFor,
  orbSpin,
  type OrbBands,
} from './orbDrive';

/** Ordinary speech reaches the shaper from the analyser bands around 0.10-0.60. */
const soft: OrbBands = { low: 0.12, mid: 0.1, high: 0.08 };
const hard: OrbBands = { low: 0.45, mid: 0.4, high: 0.3 };
const silence: OrbBands = { low: 0, mid: 0, high: 0 };
const clock = (step = 0.25, until = 6) => {
  const out: number[] = [];
  for (let t = 0; t <= until; t += step) out.push(t);
  return out;
};

describe('orbMotionFor', () => {
  it('maps voice states onto the three motion kinds', () => {
    expect(orbMotionFor('recording')).toBe('listening');
    expect(orbMotionFor('processing')).toBe('thinking');
    expect(orbMotionFor('connecting')).toBe('thinking');
    expect(orbMotionFor('playing')).toBe('speaking');
    expect(orbMotionFor('ready')).toBe('idle');
    expect(orbMotionFor('idle')).toBe('idle');
  });
});

describe('listening — the pulse is the microphone', () => {
  // The regression this change exists for: the old floor was a 0.14-0.24 sine
  // sitting ON TOP of ordinary speech, so Math.max returned the floor and the
  // orb pulsed to a metronome rather than to the speaker.
  it('distinguishes soft speech from loud at every phase of the floor', () => {
    for (const t of clock()) {
      const a = orbBands('listening', soft, t);
      const b = orbBands('listening', hard, t);
      expect(b.low - a.low).toBeGreaterThan(0.1);
    }
  });

  it('keeps its synthetic floor below ordinary speech', () => {
    for (const t of clock()) {
      expect(orbBands('listening', silence, t).low).toBeLessThanOrEqual(ORB_FLOOR_CEILING);
    }
  });

  it('still breathes at true silence', () => {
    const a = orbBands('listening', silence, 0);
    const b = orbBands('listening', silence, 0.7);
    expect(a.low).toBeGreaterThan(0.03);
    expect(Math.abs(a.low - b.low)).toBeGreaterThan(0.001);
  });
});

describe('speaking — shape follows the TTS envelope', () => {
  it('keeps a residual so a quiet syllable does not kill the orb', () => {
    expect(orbBands('speaking', silence, 0.2).low).toBeGreaterThan(0.08);
  });

  it('moves the shape monotonically with the envelope', () => {
    for (const t of clock(0.2, 2)) {
      let previous = -1;
      for (let level = 0.1; level <= 0.9; level += 0.1) {
        const amp = orbAmp(
          orbBands('speaking', { low: level, mid: level, high: level }, t).low,
          true,
        );
        expect(amp).toBeGreaterThan(previous);
        previous = amp;
      }
    }
  });

  it('swells and spins harder than the non-speaking states at the same band', () => {
    expect(orbAmp(0.6, true)).toBeGreaterThan(orbAmp(0.6, false));
    expect(orbSpin(0.6, true)).toBeGreaterThan(orbSpin(0.6, false));
  });
});

describe('thinking — a different kind of motion', () => {
  // With multiple seconds of model thinking in front of every reply, this is
  // the state the user stares at. It must not read as "still listening".
  it('turns instead of pulsing, and out-spins a loud listening frame', () => {
    const lows = clock(0.2).map((t) => orbBands('thinking', silence, t).low);
    const swing = Math.max(...lows) - Math.min(...lows);
    expect(swing).toBeLessThan(0.03);

    const listeningLows = clock(0.2).map((t) => orbBands('listening', silence, t).low);
    const listeningSwing = Math.max(...listeningLows) - Math.min(...listeningLows);
    expect(listeningSwing).toBeGreaterThan(swing);

    const mids = clock(0.2).map((t) => orbBands('thinking', silence, t).mid);
    expect(Math.min(...mids)).toBeGreaterThan(0.3);
    expect(orbSpin(Math.min(...mids), false)).toBeGreaterThan(orbSpin(0.27, false));
  });

  it('ignores the analyser entirely — a live mic must not light the wait', () => {
    expect(orbBands('thinking', hard, 1.3)).toEqual(orbBands('thinking', silence, 1.3));
  });

  it('is never perfectly frozen', () => {
    expect(orbBands('thinking', silence, 0).low).not.toBe(
      orbBands('thinking', silence, 1).low,
    );
  });
});

describe('sanitisation', () => {
  it('survives a NaN band from a mid-route analyser', () => {
    for (const motion of ['listening', 'thinking', 'speaking', 'idle'] as const) {
      const b = orbBands(motion, { low: NaN, mid: Infinity, high: -1 }, 0.4);
      expect(Number.isFinite(b.low)).toBe(true);
      expect(Number.isFinite(b.mid)).toBe(true);
      expect(Number.isFinite(b.high)).toBe(true);
      expect(b.low).toBeGreaterThanOrEqual(0);
    }
  });
});
