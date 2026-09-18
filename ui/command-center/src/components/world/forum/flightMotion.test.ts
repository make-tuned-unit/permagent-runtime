import { describe, expect, it } from 'vitest';
import {
  CEILING,
  TOUCHDOWN_HEIGHT,
  flightStep,
  gearIsDown,
  idleInput,
  thrusterGlow,
  type FlightState,
  type GroundSample,
} from './flightMotion';

const aloft = (over: Partial<FlightState> = {}): FlightState => ({
  x: 0,
  y: 60,
  z: 0,
  yaw: 0,
  vx: 0,
  vy: 0,
  vz: 0,
  landed: false,
  ...over,
});

/** A terrace at 40m: the kind of floating island the skiff should land on. */
const terrace = (): GroundSample => ({ height: 40, landable: true });
/** Open water: never landable. */
const water = (): GroundSample => ({ height: 0, landable: false });

function fly(state: FlightState, input: Parameters<typeof flightStep>[1], seconds: number, ground: () => GroundSample) {
  let current = state;
  for (let t = 0; t < seconds; t += 1 / 60) {
    current = flightStep(current, input, 1 / 60, () => ground());
  }
  return current;
}

describe('flying the skiff', () => {
  it('goes where it is pointed', () => {
    const flown = fly(aloft(), { ...idleInput, thrust: 1 }, 3, () => null);
    // Yaw 0 faces -Z, so ahead is -Z and the skiff should not drift sideways.
    expect(flown.z).toBeLessThan(-20);
    expect(Math.abs(flown.x)).toBeLessThan(1);
  });

  it('turns, and then travels along the new heading', () => {
    const turned = fly(aloft(), { ...idleInput, turn: 1 }, 1.4, () => null);
    expect(turned.yaw).toBeGreaterThan(1);
    const travelled = fly(turned, { ...idleInput, thrust: 1 }, 3, () => null);
    expect(Math.abs(travelled.x)).toBeGreaterThan(5);
  });

  it('climbs and descends, and never leaves the sky behind', () => {
    const climbed = fly(aloft({ y: CEILING - 30 }), { ...idleInput, lift: 1 }, 20, () => null);
    expect(climbed.y).toBeLessThanOrEqual(CEILING);
    const dropped = fly(aloft(), { ...idleInput, lift: -1 }, 2, () => null);
    expect(dropped.y).toBeLessThan(60);
  });

  it('boosts faster than it cruises', () => {
    const cruise = fly(aloft(), { ...idleInput, thrust: 1 }, 4, () => null);
    const boost = fly(aloft(), { ...idleInput, thrust: 1, boost: true }, 4, () => null);
    expect(Math.abs(boost.z)).toBeGreaterThan(Math.abs(cruise.z) * 1.5);
  });

  it('lands on a floating terrace when the approach is gentle', () => {
    const settling = fly(aloft({ y: 44 }), { ...idleInput, lift: -1 }, 3, terrace);
    expect(settling.landed).toBe(true);
    expect(settling.y).toBeCloseTo(40, 5);
    // A landed skiff does not creep.
    expect(Math.hypot(settling.vx, settling.vy, settling.vz)).toBe(0);
  });

  it('stops short instead of passing through the deck when arriving fast', () => {
    const fast = flightStep(aloft({ y: 40.5, vy: -40 }), { ...idleInput, lift: -1 }, 1 / 60, terrace);
    expect(fast.y).toBeGreaterThanOrEqual(40);
    // Either it settled or it was held above the deck — never below it.
    expect(fast.landed || fast.y >= 40).toBe(true);
  });

  it('refuses to land on water, and holds a clearance above it', () => {
    const overWater = fly(aloft({ y: 6 }), { ...idleInput, lift: -1 }, 4, water);
    expect(overWater.landed).toBe(false);
    expect(overWater.y).toBeGreaterThan(1.5);
  });

  it('puts the gear down on approach, not in cruise', () => {
    expect(gearIsDown(aloft({ y: 300 }), terrace())).toBe(false);
    expect(gearIsDown(aloft({ y: 45 }), terrace())).toBe(true);
    expect(gearIsDown(aloft({ y: 45 }), water())).toBe(false);
    expect(gearIsDown(aloft({ landed: true }), null)).toBe(true);
  });

  it('glows with effort and idles once down', () => {
    const working = thrusterGlow(aloft({ vx: 20, vz: 20 }), { ...idleInput, thrust: 1 });
    const idle = thrusterGlow(aloft({ landed: true }), idleInput);
    expect(working).toBeGreaterThan(idle);
    expect(working).toBeLessThanOrEqual(1);
    expect(idle).toBeGreaterThan(0);
  });

  it('is deterministic for the same inputs', () => {
    const a = fly(aloft(), { ...idleInput, thrust: 1, turn: 0.5 }, 2, terrace);
    const b = fly(aloft(), { ...idleInput, thrust: 1, turn: 0.5 }, 2, terrace);
    expect(a).toEqual(b);
  });

  it('touches down within the documented height, not at an arbitrary one', () => {
    const justAbove = flightStep(aloft({ y: 40 + TOUCHDOWN_HEIGHT - 0.1, vy: -1 }), idleInput, 1 / 60, terrace);
    expect(justAbove.landed).toBe(true);
  });
});
