/**
 * Flying the skiff, as pure arithmetic.
 *
 * Jesse asked to board a ship, fly to other parts of the map, and land on the
 * floating islands. Landing is the hard half: a flyer that cannot be put down
 * deliberately is a camera with extra steps. So the model here is a lander —
 * damped, forgiving, and willing to touch down whenever the ground beneath it
 * is solid and the approach is gentle.
 *
 * Everything is a function of a state, an input and a time step, with the world
 * supplied as a callback. That keeps the flight feel testable without a scene,
 * and means the rules about what counts as landable live in one place.
 */

/** Metres per second at full throttle, and with the boost held. */
export const CRUISE_SPEED = 26;
export const BOOST_SPEED = 62;
/** Vertical metres per second under climb or descend. */
export const LIFT_SPEED = 14;
/** Turn rate in radians per second. */
export const YAW_RATE = 1.15;
/** How quickly velocity converges on the commanded velocity (per second). */
export const RESPONSE = 2.6;
/** Below this height above a landable surface the legs come out. */
export const GEAR_HEIGHT = 12;
/** Within this height of a landable deck, the skiff brakes itself for touchdown. */
export const FLARE_HEIGHT = 9;
/** Touchdown happens under this height and this descent rate. */
export const TOUCHDOWN_HEIGHT = 1.4;
export const TOUCHDOWN_SPEED = 9;
/** The skiff never climbs past this; the sky dome is at 4800. */
export const CEILING = 900;

export type FlightInput = {
  /** -1 astern, 0 idle, +1 ahead. */
  thrust: number;
  /** -1 left, 0, +1 right. */
  turn: number;
  /** -1 down, 0, +1 up. */
  lift: number;
  boost: boolean;
};

export type FlightState = {
  x: number;
  y: number;
  z: number;
  /** Heading, radians, 0 = facing -Z, matching the model's +Y bow. */
  yaw: number;
  /** Velocity, world units per second. */
  vx: number;
  vy: number;
  vz: number;
  /** True once the skiff has settled on a surface. */
  landed: boolean;
};

export const idleInput: FlightInput = { thrust: 0, turn: 0, lift: 0, boost: false };

/**
 * What the world is under a point: the height of the nearest surface below,
 * and whether it can be landed on. Water and steep faces are not landable.
 */
export type GroundSample = { height: number; landable: boolean } | null;

const approach = (current: number, target: number, rate: number, dt: number) =>
  current + (target - current) * Math.min(1, rate * dt);

/**
 * Advance the skiff one step.
 *
 * Landing is deliberately easy to do and hard to do by accident: it needs the
 * ground to be landable, the skiff to be within a short distance of it, and the
 * descent to be gentle. Coming in fast just stops you short of the surface
 * rather than crashing, because a world you are exploring should not punish
 * you for arriving quickly.
 */
export function flightStep(
  state: FlightState,
  input: FlightInput,
  dt: number,
  sampleGround: (x: number, z: number) => GroundSample,
): FlightState {
  const step = Math.min(dt, 0.05);
  const yaw = state.yaw + input.turn * YAW_RATE * step;

  const speed = input.boost ? BOOST_SPEED : CRUISE_SPEED;
  // Heading vector for a +Y bow under three's -Z forward convention.
  const wantVx = -Math.sin(yaw) * input.thrust * speed;
  const wantVz = -Math.cos(yaw) * input.thrust * speed;
  const wantVy = input.lift * LIFT_SPEED;

  const vx = approach(state.vx, wantVx, RESPONSE, step);
  let vy = approach(state.vy, wantVy, RESPONSE, step);
  const vz = approach(state.vz, wantVz, RESPONSE, step);

  let x = state.x + vx * step;
  let y = state.y + vy * step;
  let z = state.z + vz * step;
  if (y > CEILING) {
    y = CEILING;
  }

  const ground = sampleGround(x, z);
  let landed = false;
  if (ground) {
    const deck = ground.height;
    let descending = -vy;
    // THE FLARE, and without it landing was impossible.
    //
    // Descent runs at LIFT_SPEED (14 m/s) while the key is held, but touchdown
    // requires arriving under TOUCHDOWN_SPEED (9 m/s). Hold the key and you are
    // always too fast; release it and the skiff simply stops descending,
    // because there is no gravity to carry you the last few metres. So the only
    // way down was a keypress of exactly the right length, which is not a
    // landing, it is a trick.
    //
    // Close to a deck it can settle on, the skiff now brakes itself to a
    // touchdown-legal speed, the way any lander does on final approach. Holding
    // descend over a floating island now simply lands you.
    if (ground.landable) {
      const height = y - deck;
      if (height < FLARE_HEIGHT && vy < 0) {
        const eased = Math.max(0.12, height / FLARE_HEIGHT);
        const capped = TOUCHDOWN_SPEED * 0.75 * eased;
        if (descending > capped) {
          vy = -capped;
          descending = capped;
          y = state.y + vy * step;
        }
      }
    }
    if (ground.landable && y - deck <= TOUCHDOWN_HEIGHT && descending <= TOUCHDOWN_SPEED) {
      // Settled. Zero the velocity so a landed skiff does not creep.
      return { x, y: deck, z, yaw, vx: 0, vy: 0, vz: 0, landed: true };
    }
    // Not landable, or arriving too fast: stop short rather than pass through.
    const floor = deck + (ground.landable ? 0.35 : 2.2);
    if (y < floor) {
      y = floor;
      return { x, y, z, yaw, vx, vy: Math.max(0, vy), vz, landed: false };
    }
  }

  return { x, y, z, yaw, vx, vy, vz, landed };
}

/** Whether the landing gear should be out, given the height above ground. */
export function gearIsDown(state: FlightState, ground: GroundSample): boolean {
  if (state.landed) return true;
  if (!ground || !ground.landable) return false;
  return state.y - ground.height <= GEAR_HEIGHT;
}

/** 0..1 throttle glow for the thrusters, from how hard the skiff is working. */
export function thrusterGlow(state: FlightState, input: FlightInput): number {
  if (state.landed) return 0.05;
  const planar = Math.hypot(state.vx, state.vz);
  const climbing = Math.max(0, input.lift);
  const base = Math.min(1, planar / BOOST_SPEED);
  return Math.min(1, 0.12 + base * 0.8 + climbing * 0.25);
}
