// The MESH gate — pure geometry for the portal the Sovereign walks through.
//
// Coordinates are Three.js (Y-up) and come from the authored Blender gate court
// recorded in docs/design/solar-forum/jobs/13-terrace-gate-report.md: the court
// sits on the 135-degree axis at radius 56, its ring plane is `x + z = -79.196`,
// and the outward walk direction is (-0.7071, 0, -0.7071).
//
// Nothing here renders, reads state or talks to a network. The Agora's honesty
// contract lives in `world/shared/meshStatus.ts`; this module only answers
// "where is the gate" and "did that step go through it".

export interface Point3 {
  x: number;
  y: number;
  z: number;
}

const AXIS = Math.SQRT1_2; // cos/sin of 45 degrees — the 135-degree court axis.

export const MESH_GATE = {
  /** Ring centre in world space. */
  center: [-39.597980, 4.77, -39.597980] as [number, number, number],
  /** Ring plane normal, which is also the outward walk direction (unit, horizontal). */
  normal: [-AXIS, 0, -AXIS] as [number, number, number],
  /** Ring radius and tube radius of the authored bronze ring. */
  radius: 4.6,
  tube: 0.55,
  /** Polar radius of the court/ring centre from the forum origin. */
  courtRadius: 56,
  /** The 9 m stone plaza the ring stands on. */
  courtFloorRadius: 9,
  /** Three-step dais: its top face is y=0.72 between radius 53 and 58.5. */
  dais: { top: 0.72, fromRadius: 53, toRadius: 58.5, radii: [3.8, 3.45, 3.1] },
  /** Semicircular exedra beyond the ring, centred `offset` metres along the walk axis. */
  exedra: { radius: 7, height: 5, offset: 8 },
  /** Spur bridge from the promenade to the court edge. */
  spur: { fromRadius: 38, toRadius: 47 },
  /** A crossing only counts inside this height band — the walker is well below
   *  the ring's centre, so the ring's own circle is not the right test. */
  band: [0, 6] as [number, number],
  /** How far before the ring the approach point sits. */
  approachOffset: 6,
};

// In-plane horizontal tangent: cross(normal, up). Unit length, because the
// normal is unit length and horizontal.
const TANGENT: [number, number, number] = [-MESH_GATE.normal[2], 0, MESH_GATE.normal[0]];

/** Signed distance to the ring plane. Negative inside the forum, positive beyond
 *  the gate. On the court axis it is simply `polarRadius - 56`. */
export function gateSignedDistance(p: Point3): number {
  return (
    (p.x - MESH_GATE.center[0]) * MESH_GATE.normal[0] +
    (p.z - MESH_GATE.center[2]) * MESH_GATE.normal[2]
  );
}

/** Distance from the gate's centre line, measured across the ring plane. */
export function gateLateralOffset(p: Point3): number {
  return Math.abs(
    (p.x - MESH_GATE.center[0]) * TANGENT[0] + (p.z - MESH_GATE.center[2]) * TANGENT[2]
  );
}

/**
 * Did the segment prev -> next pass through the ring?
 *
 * The test is on the point of intersection, not on the endpoints: a step that
 * crosses the plane wide of the ring, or above/below the walkable band, is not
 * a portal crossing and returns null.
 */
export function crossedGatePlane(prev: Point3, next: Point3): 'outward' | 'inward' | null {
  const before = gateSignedDistance(prev);
  const after = gateSignedDistance(next);
  const outward = before < 0 && after >= 0;
  const inward = before >= 0 && after < 0;
  if (!outward && !inward) return null;
  const t = before / (before - after);
  const hit = {
    x: prev.x + (next.x - prev.x) * t,
    y: prev.y + (next.y - prev.y) * t,
    z: prev.z + (next.z - prev.z) * t,
  };
  if (gateLateralOffset(hit) > MESH_GATE.radius) return null;
  if (hit.y < MESH_GATE.band[0] || hit.y > MESH_GATE.band[1]) return null;
  return outward ? 'outward' : 'inward';
}

/** A point on the spur about 6 m before the ring: where "focus the gate" aims
 *  and where returning from the Agora puts the walker back down. */
export function gateApproachPoint(): [number, number, number] {
  const r = MESH_GATE.courtRadius - MESH_GATE.approachOffset;
  return [-r * AXIS, 0, -r * AXIS];
}

/** Is the walker standing in the exedra beyond the ring? Used for the
 *  "E · Enter the MESH" prompt, which is how someone who stepped around the
 *  ring rather than through it still reaches the Agora. */
export function inExedra(p: Point3): boolean {
  const s = gateSignedDistance(p);
  if (s <= 0 || s > MESH_GATE.exedra.offset + MESH_GATE.exedra.radius) return false;
  return gateLateralOffset(p) <= MESH_GATE.exedra.radius;
}

/** Y rotation that turns local +Z into the outward walk direction. */
export function gateYaw(): number {
  return Math.atan2(MESH_GATE.normal[0], MESH_GATE.normal[2]);
}

/** Gate-local coordinates (local +Z = outward, y = metres above the court floor)
 *  to world space. Lets the Agora be authored in readable local numbers. */
export function gateLocalToWorld(local: [number, number, number]): [number, number, number] {
  const yaw = gateYaw();
  const s = Math.sin(yaw);
  const c = Math.cos(yaw);
  return [
    MESH_GATE.center[0] + local[0] * c + local[2] * s,
    local[1],
    MESH_GATE.center[2] - local[0] * s + local[2] * c,
  ];
}
