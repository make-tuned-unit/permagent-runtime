/**
 * orbDrive — the orb's per-frame band targets, extracted from VoiceOrb.tsx so
 * the state→motion mapping is testable without a canvas or an audio graph.
 *
 * The contract (user report, 2026-08-25): while THEY speak the orb pulses with
 * their voice; while the AGENT speaks it changes shape with the speech; and the
 * wait in between must look like neither. The three states have to read as three
 * different KINDS of motion, not one shape at three speeds.
 *
 * Bands map onto geometry in VoiceOrb.tsx: `low` → `amp`, the magnitude of the
 * noise-field displacement (the SHAPE); `mid` → spin and surface churn rate;
 * `high` → shimmer/brightness.
 *
 * WHY THE FLOORS MOVED. Both the listening and the speaking states carry a
 * synthetic floor so the orb never freezes at silence. Those floors used to sit
 * at 0.14-0.24 (listening) — i.e. ON TOP of ordinary speech, which arrives from
 * the analyser bands at roughly 0.10-0.60 — so `Math.max(band, breath)`
 * returned the breath on most frames and the orb pulsed to a metronome instead
 * of to the voice. The floor now sits below speech and only shows through in
 * real silence.
 */

/** Ceiling for any synthetic floor: real speech must always win the `max`. */
export const ORB_FLOOR_CEILING = 0.1;

export interface OrbBands {
  low: number;
  mid: number;
  high: number;
}

/** The visual states the orb distinguishes, derived from the voice state. */
export type OrbMotion = 'listening' | 'thinking' | 'speaking' | 'idle';

/** Map a voice-hook state onto the orb's motion vocabulary. */
export function orbMotionFor(state: string): OrbMotion {
  if (state === 'recording') return 'listening';
  if (state === 'processing' || state === 'connecting') return 'thinking';
  if (state === 'playing') return 'speaking';
  return 'idle';
}

/**
 * Shape the raw analyser bands for one frame.
 *
 * `raw` is the live low/mid/high split (or zeroes when no analyser is
 * available); `tSec` is a monotonic clock in seconds, used only for the
 * synthetic floors.
 */
export function orbBands(motion: OrbMotion, raw: OrbBands, tSec: number): OrbBands {
  const sane = (v: number) => (Number.isFinite(v) ? Math.min(1.5, Math.max(0, v)) : 0);
  const low = sane(raw.low);
  const mid = sane(raw.mid);
  const high = sane(raw.high);

  if (motion === 'thinking') {
    // Turning, not breathing. `low` is held almost flat so the surface does
    // not pulse, while `mid` is pinned high so the orb visibly ROTATES and
    // churns. With multiple seconds of model thinking in front of every reply
    // this is the state the user stares at, and it has to say "working" — not
    // "idle", and above all not "still listening to you".
    return {
      low: 0.085 + 0.005 * Math.sin(tSec * 0.9),
      mid: 0.42 + 0.05 * Math.sin(tSec * 0.7),
      high: 0.02,
    };
  }

  if (motion === 'listening') {
    // The pulse IS the microphone. The floor only shows through at silence.
    const floor = 0.05 + 0.03 * (0.5 + 0.5 * Math.sin(tSec * 2.2));
    return {
      low: Math.max(floor, low),
      mid: Math.max(floor * 0.7, mid),
      high: Math.max(floor * 0.4, high),
    };
  }

  if (motion === 'speaking') {
    // Shape follows the TTS envelope. The residual is a FLOOR, not a blend: a
    // quiet syllable must not be papered over by a sine, or the orb stops
    // being the agent's voice and becomes decoration.
    const residual = 0.09;
    return {
      low: Math.max(low * 1.35, residual),
      mid: Math.max(mid * 1.35, residual * 0.7),
      high: Math.max(high * 1.35, residual * 0.5),
    };
  }

  const breath = 0.09 + 0.06 * (0.5 + 0.5 * Math.sin(tSec * 1.5));
  return {
    low: Math.max(low, breath),
    mid: Math.max(mid, breath * 0.65),
    high: Math.max(high, breath * 0.35),
  };
}

/** Radial displacement magnitude — the orb's shape change. */
export function orbAmp(low: number, speaking: boolean): number {
  return speaking ? 0.055 + low * 0.5 : 0.045 + low * 0.34;
}

/** Rotation rate. */
export function orbSpin(mid: number, speaking: boolean): number {
  return speaking ? 0.28 + mid * 1.25 : 0.2 + mid * 0.9;
}
