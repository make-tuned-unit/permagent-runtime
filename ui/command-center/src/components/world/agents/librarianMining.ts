// Librarian mining/describe loop — THE_CAVE_vision_bible.md §5 (Brain/Archive):
// "The Librarian rides the gantry: pull a dim tablet, it brightens violet in its hands
// (a real describe event), reshelve it glowing." This module is the loop's state, driven
// ONLY by real LibrarianDescribe* events (noteDescribe is called from stateSources.tsx's
// /events binding; replayed history is filtered there). No event ⇒ no tablet (the rig
// shows the tablet only while mining).
//
// Phases of one quarried stone:
//   idle       — no tablet in hand
//   pull       — describe_started: a dim tablet detaches from the shelf into the hands
//   brighten   — token/duration of the describe: the tablet's violet emissive ramps up
//   reshelve   — describe_completed: the now-glowing tablet returns to the shelf
//   error      — describe failed: the tablet dims and is set down (no glow earned)
//
// Continuous animation reads getMiningState() inside the Librarian's useFrame; discrete
// transitions are set here from the real events. Zero per-frame allocations.

export type MiningPhase = 'idle' | 'pull' | 'brighten' | 'reshelve' | 'error';

export interface MiningState {
  phase: MiningPhase;
  /** Epoch ms the current phase started — drives the tablet ramp/return easing. */
  since: number;
  /** Memory key being described (evidence trail; shown in the dev ledger). */
  ref: string;
  /** Completed tablets reshelved this session (the glow count). */
  reshelved: number;
}

// Phase durations (ms). The brighten phase is open-ended (held until completed); these
// only bound the pull-in and reshelve-out tweens so they read as deliberate moves.
export const PULL_MS = 900;
export const RESHELVE_MS = 1100;
export const ERROR_SETDOWN_MS = 900;

const state: MiningState = { phase: 'idle', since: 0, ref: '', reshelved: 0 };

/** Real-event entry point. Called from the /events binding (live events only). */
export function noteDescribe(kind: 'start' | 'complete' | 'error', ref: string): void {
  const now = Date.now();
  switch (kind) {
    case 'start':
      // A new describe begins — pull a fresh dim tablet (even if mid-reshelve).
      state.phase = 'pull';
      state.since = now;
      state.ref = ref;
      break;
    case 'complete':
      if (state.phase === 'idle') return; // completion with no live start — ignore
      state.phase = 'reshelve';
      state.since = now;
      state.reshelved += 1;
      break;
    case 'error':
      if (state.phase === 'idle') return;
      state.phase = 'error';
      state.since = now;
      break;
  }
}

/** Non-hook read for the Librarian's useFrame. */
export function getMiningState(): MiningState {
  return state;
}

/**
 * Resolve the tablet's visible parameters for the current frame.
 *   visible  — render the tablet at all
 *   reach    — 0 (at shelf) … 1 (fully in hands); reshelve/error return toward 0
 *   glow     — 0 (dim) … 1 (full violet); ramps during brighten, holds on reshelve
 * Pure scalar math (caller passes `now` from the frame clock). No allocations.
 */
export function resolveTablet(now: number): { visible: boolean; reach: number; glow: number } {
  const t = now - state.since;
  switch (state.phase) {
    case 'idle':
      return { visible: false, reach: 0, glow: 0 };
    case 'pull': {
      const k = Math.min(1, t / PULL_MS);
      return { visible: true, reach: k, glow: 0.12 * k };
    }
    case 'brighten': {
      // Open-ended ramp toward full glow while the describe runs.
      const k = Math.min(1, t / 4000);
      return { visible: true, reach: 1, glow: 0.12 + 0.88 * k };
    }
    case 'reshelve': {
      const k = Math.min(1, t / RESHELVE_MS);
      if (k >= 1) {
        state.phase = 'idle';
        state.since = now;
        return { visible: false, reach: 0, glow: 1 };
      }
      // Returns to the shelf glowing — reach falls, glow stays full.
      return { visible: true, reach: 1 - k, glow: 1 };
    }
    case 'error': {
      const k = Math.min(1, t / ERROR_SETDOWN_MS);
      if (k >= 1) {
        state.phase = 'idle';
        state.since = now;
        return { visible: false, reach: 0, glow: 0 };
      }
      // Tablet dims and is set down — no glow earned.
      return { visible: true, reach: 1 - k, glow: 0.12 * (1 - k) };
    }
  }
}

/** Advance pull → brighten once the pull-in tween completes. Called from useFrame. */
export function advanceMining(now: number): void {
  if (state.phase === 'pull' && now - state.since >= PULL_MS) {
    state.phase = 'brighten';
    state.since = now;
  }
}
