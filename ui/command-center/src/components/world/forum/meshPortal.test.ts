import { describe, expect, it } from 'vitest';
import {
  MESH_GATE,
  crossedGatePlane,
  gateApproachPoint,
  gateLocalToWorld,
  gateSignedDistance,
  inExedra,
} from './meshPortal';

/** A point on the 135-degree court axis at the given polar radius. */
function onAxis(radius: number, y = 0.72) {
  const [x, , z] = gateLocalToWorld([0, y, radius - MESH_GATE.courtRadius]);
  return { x, y, z };
}
/** A point `across` metres to one side of the gate's centre line. */
function offAxis(radius: number, across: number, y = 0.72) {
  const [x, , z] = gateLocalToWorld([across, y, radius - MESH_GATE.courtRadius]);
  return { x, y, z };
}

describe('the MESH gate plane', () => {
  it('places the ring where the exported gate court put it', () => {
    // The authored plane is x + z = -79.195959; on the court axis the signed
    // distance is simply the polar radius minus 56.
    expect(gateSignedDistance(onAxis(56))).toBeCloseTo(0, 4);
    expect(gateSignedDistance(onAxis(50))).toBeCloseTo(-6, 4);
    expect(gateSignedDistance(onAxis(64))).toBeCloseTo(8, 4);
  });

  it('reports an outward crossing when a step goes through the ring', () => {
    expect(crossedGatePlane(onAxis(54), onAxis(58))).toBe('outward');
    // A step that lands exactly on the plane still counts as through it.
    expect(crossedGatePlane(onAxis(55.8), onAxis(56))).toBe('outward');
  });

  it('reports an inward crossing when the same step is taken back', () => {
    expect(crossedGatePlane(onAxis(58), onAxis(54))).toBe('inward');
  });

  it('ignores a crossing that passes wide of the ring', () => {
    // 6 m off the centre line is past the 4.6 m ring radius — that is walking
    // round the side of the gate, not through it.
    expect(crossedGatePlane(offAxis(54, 6), offAxis(58, 6))).toBeNull();
    expect(crossedGatePlane(offAxis(54, 4), offAxis(58, 4))).toBe('outward');
  });

  it('ignores a crossing outside the walkable height band', () => {
    expect(crossedGatePlane(onAxis(54, 9), onAxis(58, 9))).toBeNull();
  });

  it('returns null when the step stays on one side', () => {
    expect(crossedGatePlane(onAxis(50), onAxis(54))).toBeNull();
    expect(crossedGatePlane(onAxis(60), onAxis(64))).toBeNull();
    expect(crossedGatePlane(onAxis(56), onAxis(56))).toBeNull();
  });

  it('puts the approach point six metres in front of the ring, on the spur', () => {
    const [x, y, z] = gateApproachPoint();
    expect(gateSignedDistance({ x, y, z })).toBeCloseTo(-MESH_GATE.approachOffset, 4);
    expect(Math.hypot(x, z)).toBeCloseTo(MESH_GATE.courtRadius - MESH_GATE.approachOffset, 4);
    expect(y).toBe(0);
    // Inside the spur/court run the walk route covers (r=36 to r=63).
    expect(Math.hypot(x, z)).toBeGreaterThan(MESH_GATE.spur.toRadius);
  });

  it('knows when the walker is standing in the exedra beyond the ring', () => {
    expect(inExedra(onAxis(64))).toBe(true);
    expect(inExedra(offAxis(62, 5.5))).toBe(true);
    expect(inExedra(offAxis(62, 8))).toBe(false); // outside the 7 m exedra
    expect(inExedra(onAxis(54))).toBe(false); // still in the forum
    expect(inExedra(onAxis(80))).toBe(false); // off the end of the floor
  });
});
