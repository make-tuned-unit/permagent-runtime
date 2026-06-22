// Agent motion store — continuous locomotion lives OUTSIDE React state.
// WORLD_VIEW_BIBLE.md §8: zero per-frame allocations, no RAF+setState-per-frame.
// Positions/headings are mutable records advanced once per frame from a single
// useFrame in WorldAgents; characters read them via getMotion() inside useFrame.
// Discrete events (arrival) fire callbacks so React state only changes discretely.
//
// Locomotion is straight-line + ease between waypoints. NO pathfinding engine
// (explicit scope fence, bible §4).
//
// Walkable cave (rebuild §5c): an agent's Y tracks the floor/ramp under it via the
// analytic floorYAt — this is what makes the cave a walkable 3D volume rather than a
// flat plane. Applied each frame to every free-roaming agent; the ring-locked Librarian
// (mezzanine) keeps its own elevation.

import { getStrata } from '../areas/strata/strataState';
import { floorYAt, WELL_RADIUS, THROAT } from '../areas/strata/strata';

export interface Waypoint {
  x: number;
  y: number;
  z: number;
  /** Y rotation the agent assumes on arrival at this waypoint. */
  facing?: number;
  /** Pause at this waypoint before continuing to the next one. */
  pauseMs?: number;
}

export type Engagement = 'none' | 'seated' | 'standing' | 'tending';

export interface MotionState {
  x: number;
  y: number;
  z: number;
  /** Current yaw (radians). */
  heading: number;
  targetHeading: number;
  walking: boolean;
  engaged: Engagement;
  queue: Waypoint[];
  waitUntil: number; // epoch ms; pausing at a waypoint when > now
  onArrive: (() => void) | null;
  /** Librarian: project XZ onto this radius (mezzanine ring lock). */
  ringLock: number | null;
}

const WALK_SPEED = 3; // u/s — existing system speed
const ARRIVE_DIST = 0.2;
const TURN_RATE = 8; // rad/s damping factor

const store = new Map<string, MotionState>();

export function ensureMotion(
  id: string,
  home: { x: number; y: number; z: number },
  ringLock: number | null = null,
): MotionState {
  let m = store.get(id);
  if (!m) {
    m = {
      x: home.x,
      y: home.y,
      z: home.z,
      heading: 0,
      targetHeading: 0,
      walking: false,
      engaged: 'none',
      queue: [],
      waitUntil: 0,
      onArrive: null,
      ringLock,
    };
    store.set(id, m);
  }
  return m;
}

export function getMotion(id: string): MotionState | undefined {
  return store.get(id);
}

/** For W4 camera integration: read an agent's live position without React state. */
export function getAgentPosition(id: string): { x: number; y: number; z: number } | null {
  const m = store.get(id);
  return m ? { x: m.x, y: m.y, z: m.z } : null;
}

export function setPath(id: string, waypoints: Waypoint[], onArrive?: () => void): void {
  const m = store.get(id);
  if (!m) return;
  m.queue = waypoints.slice();
  m.onArrive = onArrive ?? null;
  m.waitUntil = 0;
  m.walking = m.queue.length > 0;
}

/**
 * User puppeting (W4 third-person — arrow keys / WASD). Nudges the selected agent
 * directly and drops any autonomous path so the manual drive isn't fought by
 * advanceMotion on the next frame. Honors the Librarian's mezzanine ring lock.
 *
 * NOTE (bible §4 autonomy fence): this is deliberate per-user control — while the
 * user holds the keys the agent's autonomous locomotion is overridden. It resumes
 * autonomously once a new path is assigned by the behavior/state sources.
 */
export function nudgeAgent(id: string, dx: number, dz: number): void {
  const m = store.get(id);
  if (!m) return;
  // Take the wheel: clear the autonomous queue so the nudge sticks.
  m.queue.length = 0;
  m.onArrive = null;
  m.waitUntil = 0;
  m.engaged = 'none';
  m.x += dx;
  m.z += dz;
  if (dx !== 0 || dz !== 0) {
    m.targetHeading = Math.atan2(dx, dz);
    m.walking = true;
  }
  // Mezzanine ring lock — never leave the ring (Librarian).
  if (m.ringLock !== null) {
    const r = Math.sqrt(m.x * m.x + m.z * m.z);
    if (r > 0.1) {
      m.x = (m.x / r) * m.ringLock;
      m.z = (m.z / r) * m.ringLock;
    }
  }
}

export function stopAgent(id: string): void {
  const m = store.get(id);
  if (!m) return;
  m.queue.length = 0;
  m.onArrive = null;
  m.walking = false;
}

export function setEngaged(id: string, engaged: Engagement): void {
  const m = store.get(id);
  if (m) m.engaged = engaged;
}

function shortestAngle(a: number): number {
  while (a > Math.PI) a -= Math.PI * 2;
  while (a < -Math.PI) a += Math.PI * 2;
  return a;
}

/** Advance every agent. Called once per frame; scalar math only (no allocations). */
export function advanceMotion(dt: number): void {
  const now = Date.now();
  const strata = getStrata();
  for (const m of store.values()) {
    // Heading damping always runs (also used for arrival facing).
    m.heading += shortestAngle(m.targetHeading - m.heading) * Math.min(1, TURN_RATE * dt);

    // Walkable cave floor-follow (rebuild §5c): set Y from the surface under the agent.
    // Runs for nudged (queue-cleared), idle, and moving agents; the ring-locked
    // Librarian keeps its mezzanine elevation.
    if (m.ringLock === null) {
      // Soft well-clamp: once descended, don't walk off into the open throat shaft
      // (v1; §7 stretch = a real fall). Push back to the rim.
      if (m.y < -0.5) {
        const wx = m.x - THROAT.center[0];
        const wz = m.z - THROAT.center[2];
        const wr = Math.sqrt(wx * wx + wz * wz);
        const minR = WELL_RADIUS + 0.6;
        if (wr > 0.0001 && wr < minR) {
          m.x = THROAT.center[0] + (wx / wr) * minR;
          m.z = THROAT.center[2] + (wz / wr) * minR;
        }
      }
      m.y = floorYAt(m.x, m.z, m.y, strata);
    }

    if (m.queue.length === 0) {
      m.walking = false;
      continue;
    }
    if (m.waitUntil > now) {
      m.walking = false;
      continue;
    }

    const wp = m.queue[0];
    const dx = wp.x - m.x;
    const dz = wp.z - m.z;
    const dist = Math.sqrt(dx * dx + dz * dz);

    if (dist < ARRIVE_DIST) {
      m.x = wp.x;
      m.z = wp.z;
      m.y = wp.y;
      if (wp.facing !== undefined) m.targetHeading = wp.facing;
      m.queue.shift();
      if (wp.pauseMs) {
        m.waitUntil = now + wp.pauseMs;
        m.walking = false;
      }
      if (m.queue.length === 0) {
        m.walking = false;
        const cb = m.onArrive;
        m.onArrive = null;
        if (cb) cb();
      }
      continue;
    }

    m.walking = true;
    m.targetHeading = Math.atan2(dx, dz);
    const step = Math.min(WALK_SPEED * dt, dist);
    m.x += (dx / dist) * step;
    m.z += (dz / dist) * step;
    // Vertical: ring-locked movers (the mezzanine Librarian) ease toward the
    // waypoint's y; for everyone else cave floor-follow (top of loop) owns Y.
    if (m.ringLock !== null) {
      m.y += (wp.y - m.y) * Math.min(1, step / Math.max(dist, 0.001));
    }

    // Mezzanine ring lock — never cut across the void.
    if (m.ringLock !== null) {
      const r = Math.sqrt(m.x * m.x + m.z * m.z);
      if (r > 0.1) {
        m.x = (m.x / r) * m.ringLock;
        m.z = (m.z / r) * m.ringLock;
      }
    }
  }
}
