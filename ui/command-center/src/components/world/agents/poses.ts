// Body-language pose tables — WORLD_VIEW_BIBLE.md §4 (LAW).
// State is expressed ONLY through the state channels (visor + joint glow rings +
// cape circuit lines + feet aura) and these postures. Identity trim never changes.
// Transitions tween over 0.8s (color lerp + posture blend) — no snapping.

import { STATE, type AgentHudState } from '../shared/palette';

export const BONE_NAMES = [
  'root',
  'spine',
  'head',
  'armL',
  'foreL',
  'armR',
  'foreR',
  'thighL',
  'calfL',
  'thighR',
  'calfR',
  'aura',
] as const;

export type BoneName = (typeof BONE_NAMES)[number];

export type PoseKey = 'idle' | 'available' | 'seatedWork' | 'standWork' | 'error';

export interface Pose {
  /** Root bone Y offset — seated postures drop the body (seated height ~1.7u). */
  rootY: number;
  /** Euler XYZ targets per bone; unlisted bones return to identity. */
  rot: Partial<Record<BoneName, [number, number, number]>>;
}

export const POSES: Record<PoseKey, Pose> = {
  // Standing, slow ambient sway (applied additively), occasional weight shift.
  idle: {
    rootY: 0,
    rot: {},
  },
  // Alert stance: straightened spine, head up.
  available: {
    rootY: 0,
    rot: {
      spine: [-0.05, 0, 0],
      head: [-0.08, 0, 0],
    },
  },
  // Seated/engaged at a W2 anchor, leaning in, hands toward the work surface.
  seatedWork: {
    rootY: -0.5,
    rot: {
      spine: [0.26, 0, 0],
      head: [0.12, 0, 0],
      armL: [-0.95, 0, -0.15],
      armR: [-0.95, 0, 0.15],
      foreL: [-0.35, 0, 0],
      foreR: [-0.35, 0, 0],
      thighL: [-1.4, 0, 0],
      thighR: [-1.4, 0, 0],
      calfL: [1.25, 0, 0],
      calfR: [1.25, 0, 0],
    },
  },
  // Working with no seat anchor (Librarian on the mezzanine): standing lean-in.
  standWork: {
    rootY: 0,
    rot: {
      spine: [0.3, 0, 0],
      head: [0.18, 0, 0],
      armL: [-0.85, 0, -0.1],
      armR: [-0.85, 0, 0.1],
      foreL: [-0.3, 0, 0],
      foreR: [-0.3, 0, 0],
    },
  },
  // The slump: shoulders dropped, head bowed 20° (0.35 rad).
  error: {
    rootY: -0.06,
    rot: {
      spine: [0.14, 0, 0],
      head: [0.35, 0, 0],
      armL: [0.08, 0, 0.28],
      armR: [0.08, 0, -0.28],
      foreL: [0.06, 0, 0],
      foreR: [0.06, 0, 0],
    },
  },
};

export interface StateVisual {
  color: string;
  /** Emissive intensity for the visor channel. */
  visorIntensity: number;
  /** Emissive intensity for joint rings / cape circuits / feet aura. */
  stateIntensity: number;
}

/** §4 color channel table. Visor dim 0.3 idle, bright working, full available. */
export const STATE_VISUALS: Record<AgentHudState, StateVisual> = {
  idle: { color: STATE.idle, visorIntensity: 0.3, stateIntensity: 0.35 },
  working: { color: STATE.working, visorIntensity: 1.6, stateIntensity: 1.3 },
  available: { color: STATE.available, visorIntensity: 2.0, stateIntensity: 1.0 },
  // Error visor flickers at 2Hz for 3s then holds steady dim (handled per-frame).
  error: { color: STATE.error, visorIntensity: 0.5, stateIntensity: 0.8 },
};

/** Posture transition + color lerp duration (bible §4). */
export const TRANSITION_S = 0.8;
/** Error visor flicker: 2Hz for 3s then steady dim. */
export const ERROR_FLICKER_HZ = 2;
export const ERROR_FLICKER_S = 3;
