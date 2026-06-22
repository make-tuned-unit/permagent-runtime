// Agent motion store — continuous locomotion lives OUTSIDE React state.
// WORLD_VIEW_BIBLE.md §8: zero per-frame allocations, no RAF+setState-per-frame.
// Positions/headings are mutable records advanced once per frame from a single
// useFrame in WorldAgents; characters read them via getMotion() inside useFrame.
// Discrete events (arrival) fire callbacks so React state only changes discretely.
//
// Locomotion is straight-line + ease between waypoints. NO pathfinding engine
// (explicit scope fence, bible §4).

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

// Ground agents stay ON the rotunda floor (radius 15) — they can't walk off into the
// void. Agents up on the stairs/mezzanine (y >= STAIR_FLOOR_Y) are exempt, as is the
// ring-locked Librarian (it has its own projection).
const EDGE_R = 14.2;
const STAIR_FLOOR_Y = 5;

/** Keep a free-roaming ground agent within the rotunda surface. Scalar, no allocation. */
function clampToRotunda(m: MotionState): void {
  if (m.ringLock !== null || m.y >= STAIR_FLOOR_Y) return;
  const r = Math.sqrt(m.x * m.x + m.z * m.z);
  if (r > EDGE_R) {
    m.x = (m.x / r) * EDGE_R;
    m.z = (m.z / r) * EDGE_R;
  }
}

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
  // Can't be puppeted off the rotunda edge.
  clampToRotunda(m);
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
  for (const m of store.values()) {
    // Heading damping always runs (also used for arrival facing).
    m.heading += shortestAngle(m.targetHeading - m.heading) * Math.min(1, TURN_RATE * dt);

    // Keep ground agents on the rotunda (runs for nudged/idle/moving agents alike).
    clampToRotunda(m);

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
    // Vertical: ease toward waypoint y proportionally to horizontal progress.
    m.y += (wp.y - m.y) * Math.min(1, step / Math.max(dist, 0.001));

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
