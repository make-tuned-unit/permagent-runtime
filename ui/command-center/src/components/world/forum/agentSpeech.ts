/**
 * Drives the World avatar's mouth from real speech timing.
 *
 * Inputs per frame: which word-shape the server's word timings say is being
 * heard right now (mouthVisemes.ts), and how loud the actual PCM is at that
 * instant (voicePlaybackClock.ts). Shape comes from timing; energy only
 * scales openness and forces a close in the gaps. A shape is held at least
 * MIN_SHAPE_HOLD_MS so 60 fps never turns into flicker.
 */
import {
  activeSegment,
  normalisedEnergyAt,
  type PlaybackSegment,
} from '../../../lib/voicePlaybackClock';
import {
  buildMouthTrack,
  MIN_SHAPE_HOLD_MS,
  MOUTH_SHAPES,
  shapeAt,
  type MouthShape,
  type MouthSpan,
} from './mouthVisemes';

/** Below this normalised energy the mouth closes (between words, at pauses). */
const CLOSE_ENERGY = 0.1;
/** Energy must climb past this to leave `closed` (hysteresis against chatter). */
const OPEN_ENERGY = 0.18;
/** Shortest a shape stays before the audio falling silent may close it. */
const CLOSE_MIN_HOLD_MS = 40;

const trackCache = new WeakMap<PlaybackSegment, MouthSpan[]>();

export interface MouthSample {
  /** A segment is audible right now. */
  active: boolean;
  /** Shape from word timing; null when the segment carries no word timings. */
  shape: MouthShape | null;
  /** Normalised loudness 0..1 of the actual PCM now. */
  energy: number;
}

/** What the mouth should do at `nowMs` according to the playback clock. */
export function sampleMouth(nowMs: number): MouthSample {
  const seg = activeSegment(nowMs);
  if (!seg) return { active: false, shape: null, energy: 0 };
  const t = nowMs - seg.startedAtMs;
  let track = trackCache.get(seg);
  if (!track) {
    track = buildMouthTrack(seg.words);
    trackCache.set(seg, track);
  }
  return {
    active: true,
    shape: track.length ? shapeAt(track, t) : null,
    energy: normalisedEnergyAt(seg, t),
  };
}

/** Energy-only shape for audio with no word timings. */
export function shapeFromEnergy(energy: number): MouthShape {
  if (energy < CLOSE_ENERGY) return 'closed';
  if (energy < 0.3) return 'small';
  if (energy < 0.55) return 'mid';
  if (energy < 0.8) return 'round';
  return 'wide';
}

export interface MouthState {
  shape: MouthShape;
  shapeSince: number;
  energy: number;
  w: number;
  h: number;
}

export function createMouthState(): MouthState {
  return { shape: 'closed', shapeSince: -Infinity, energy: 0, w: MOUTH_SHAPES.closed.w, h: MOUTH_SHAPES.closed.h };
}

/** Advance the mouth one frame. Returns the (mutated) state. */
export function stepMouth(
  m: MouthState,
  dt: number,
  nowMs: number,
  input: { speaking: boolean; shape: MouthShape | null; energy: number },
): MouthState {
  const d = Math.min(Math.max(dt, 0), 0.1);
  // Energy: ~15 ms attack, ~60 ms release.
  const e = input.speaking ? input.energy : 0;
  m.energy += (e - m.energy) * (1 - Math.exp(-d / (e > m.energy ? 0.015 : 0.06)));

  let wanted: MouthShape;
  if (!input.speaking) wanted = 'closed';
  else if (m.shape === 'closed' ? input.energy < OPEN_ENERGY : input.energy < CLOSE_ENERGY) wanted = 'closed';
  else wanted = input.shape ?? shapeFromEnergy(input.energy);

  // Rest closes at once; anything else waits out the minimum hold.
  const held = nowMs - m.shapeSince;
  // A pause in the audio may close the mouth a little early; opening waits the full hold.
  const mayChange = !input.speaking || held >= MIN_SHAPE_HOLD_MS || (wanted === 'closed' && held >= CLOSE_MIN_HOLD_MS);
  if (wanted !== m.shape && mayChange) {
    m.shape = wanted;
    m.shapeSince = nowMs;
  }

  const base = MOUTH_SHAPES[m.shape];
  const open = m.shape === 'closed' ? 1 : 0.55 + 0.45 * Math.min(1, m.energy / 0.8);
  const targetH = m.shape === 'closed' ? base.h : base.h * open;
  const targetW = m.shape === 'round' ? base.w : base.w * (0.88 + 0.12 * open);
  const kOpen = 1 - Math.exp(-d * 38);
  const kClose = 1 - Math.exp(-d * 26);
  m.h += (targetH - m.h) * (targetH > m.h ? kOpen : kClose);
  m.w += (targetW - m.w) * kOpen;
  return m;
}

/** Adaptive normalisation for the analyser fallback (audio with no timing metadata). */
export function createAnalyserEnergy() {
  let peak = 0.05;
  const buf = new Float32Array(256);
  return (analyser: AnalyserNode | null): number => {
    if (!analyser) return 0;
    const data = buf.length >= analyser.fftSize ? buf.subarray(0, analyser.fftSize) : new Float32Array(analyser.fftSize);
    analyser.getFloatTimeDomainData(data);
    let sum = 0;
    for (let i = 0; i < data.length; i++) sum += data[i] * data[i];
    const rms = Math.sqrt(sum / Math.max(1, data.length));
    peak = Math.max(rms, peak * 0.995, 0.03);
    return Math.min(1, rms / peak);
  };
}
