/**
 * Where each ship is, and how it is leaning, at a given moment.
 *
 * Pure and frame-rate independent, like `skyMotion.ts`: the scene passes
 * elapsed seconds and gets a pose back, so the same second always produces the
 * same frame and a dropped frame never accumulates drift.
 *
 * The boats already in the island GLB are baked into the terrain and cannot
 * move. These are separate instances sailing real courses on the lagoon, which
 * is what Jesse asked for when he asked for more ships and water that lives.
 */

/** Ships stay inside this radius of the island centre, clear of the quays. */
export const LAGOON_RADIUS = 96;
/** The waterline in world units. The hull is modelled with z=0 at the waterline. */
export const WATERLINE_Y = 0.4;

export type ShipPose = {
  x: number;
  y: number;
  z: number;
  /** Heading in radians, 0 = sailing towards -Z, matching the model's +Y bow. */
  yaw: number;
  /** Roll, radians. A ship leans away from a turn and rocks on the swell. */
  roll: number;
  /** Pitch, radians — the bow lifting and dropping over the swell. */
  pitch: number;
  /** 0..1, how hard it is sailing; the wake fades with it. */
  speed: number;
};

/**
 * One ship's pose. Each index gets its own ellipse, period and phase, so the
 * fleet never moves as a formation — the thing that makes cloned props read as
 * cloned props.
 */
export function shipCourse(seconds: number, index: number): ShipPose {
  const period = 118 + index * 27;
  const phase = index * 2.4;
  const t = (seconds / period) * Math.PI * 2 + phase;

  // An ellipse, tilted per ship, keeps the courses from nesting like rings.
  const radiusX = LAGOON_RADIUS * (0.42 + ((index * 0.17) % 0.34));
  const radiusZ = LAGOON_RADIUS * (0.38 + ((index * 0.23) % 0.3));
  const tilt = index * 1.1;
  const cx = Math.cos(t) * radiusX;
  const cz = Math.sin(t) * radiusZ;
  const x = cx * Math.cos(tilt) - cz * Math.sin(tilt);
  const z = cx * Math.sin(tilt) + cz * Math.cos(tilt);

  // Heading is the tangent of the path, so a ship always points where it goes.
  const dx = -Math.sin(t) * radiusX;
  const dz = Math.cos(t) * radiusZ;
  const tx = dx * Math.cos(tilt) - dz * Math.sin(tilt);
  const tz = dx * Math.sin(tilt) + dz * Math.cos(tilt);
  const yaw = Math.atan2(tx, tz);

  // Swell: two slow sines an octave apart, offset per ship so they bob out of
  // step with each other.
  const swell = Math.sin(seconds * 0.55 + phase) * 0.06 + Math.sin(seconds * 0.91 + phase * 1.7) * 0.03;
  // Leaning into the turn — the curvature of an ellipse peaks at its ends.
  const heel = Math.cos(t) * 0.07;
  const speed = 0.55 + Math.sin(t * 2 + phase) * 0.2;

  return {
    x,
    y: WATERLINE_Y + swell * 0.8,
    z,
    yaw,
    roll: heel + swell * 0.5,
    pitch: swell * 0.35,
    speed: Math.max(0, Math.min(1, speed)),
  };
}

/** How many ships sail the lagoon. Small: they are silhouettes at this range. */
export const SHIP_COUNT = 5;
