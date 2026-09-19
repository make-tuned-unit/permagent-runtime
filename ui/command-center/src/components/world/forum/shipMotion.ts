/**
 * Where each ship is, and how it is leaning, at a given moment.
 *
 * Pure and frame-rate independent, like `skyMotion.ts`: the scene passes
 * elapsed seconds and gets a pose back, so the same second always produces the
 * same frame and a dropped frame never accumulates drift.
 *
 * The boats already in the island GLB are baked into the terrain and cannot
 * move. These are separate instances sailing real courses on the lagoon, which
 * is what the user asked for when he asked for more ships and water that lives.
 */

/**
 * The band the lagoon actually occupies, in metres from the island centre.
 *
 * Read off the landform builder's own report (`FORUM_LANDFORM`,
 * `lagoonRadiusRange_m: [62, 118]`) rather than guessed. The first version of
 * this file used a single radius of 96 and scaled courses down from it, which
 * put most of the fleet at 36-73m — well inside the plaza — and boats sailed
 * across the ground. A course is only accepted now if the world says every
 * point along it is water (see `fitCourse`).
 */
export const LAGOON_INNER = 66;
export const LAGOON_OUTER = 114;
/** Kept for callers that only need a bound on how far a ship can wander. */
export const LAGOON_RADIUS = LAGOON_OUTER;
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
export type Course = {
  /** Metres from the island centre, along the ship's two axes. */
  radiusX: number;
  radiusZ: number;
  /** Rotation of the ellipse, radians. */
  tilt: number;
  /** Seconds for one circuit. */
  period: number;
  /** Where on the ellipse the ship starts. */
  phase: number;
};

/** The course a ship takes when nothing has told us where the water is. */
export function defaultCourse(index: number): Course {
  const span = LAGOON_OUTER - LAGOON_INNER;
  return {
    radiusX: LAGOON_INNER + span * (0.25 + ((index * 0.17) % 0.5)),
    radiusZ: LAGOON_INNER + span * (0.2 + ((index * 0.23) % 0.55)),
    tilt: index * 1.1,
    period: 118 + index * 27,
    phase: index * 2.4,
  };
}

/**
 * Walk a course's whole circuit and report whether every sample is water.
 *
 * `isWater` comes from the live scene, so this is the world answering rather
 * than a constant asserting. Sampling the whole loop matters: a course can
 * start over the lagoon and cross a terrace a quarter of the way round, which
 * is exactly the failure that put boats on the plaza.
 */
export function courseIsAfloat(
  course: Course,
  isWater: (x: number, z: number) => boolean,
  samples = 48,
): boolean {
  for (let i = 0; i < samples; i += 1) {
    const t = (i / samples) * Math.PI * 2;
    const cx = Math.cos(t) * course.radiusX;
    const cz = Math.sin(t) * course.radiusZ;
    const x = cx * Math.cos(course.tilt) - cz * Math.sin(course.tilt);
    const z = cx * Math.sin(course.tilt) + cz * Math.cos(course.tilt);
    if (!isWater(x, z)) return false;
  }
  return true;
}

/**
 * The first course in a deterministic search that stays on water, or `null`
 * when this lagoon has no room for one.
 *
 * Returning null is a real answer: a ship with nowhere to sail should not be
 * shown sailing. The search is deterministic so the fleet is identical on every
 * load, which keeps screenshots and tests comparable.
 */
export function fitCourse(
  index: number,
  isWater: (x: number, z: number) => boolean,
): Course | null {
  const base = defaultCourse(index);
  for (let shrink = 0; shrink < 9; shrink += 1) {
    for (let turn = 0; turn < 8; turn += 1) {
      const candidate: Course = {
        ...base,
        radiusX: base.radiusX - shrink * 3,
        radiusZ: base.radiusZ - shrink * 3,
        tilt: base.tilt + (turn * Math.PI) / 8,
      };
      if (candidate.radiusX < LAGOON_INNER * 0.5) break;
      if (courseIsAfloat(candidate, isWater)) return candidate;
    }
  }
  return null;
}

export function shipCourse(seconds: number, index: number, course?: Course): ShipPose {
  const fitted = course ?? defaultCourse(index);
  const period = fitted.period;
  const phase = fitted.phase;
  const t = (seconds / period) * Math.PI * 2 + phase;

  const radiusX = fitted.radiusX;
  const radiusZ = fitted.radiusZ;
  const tilt = fitted.tilt;
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
