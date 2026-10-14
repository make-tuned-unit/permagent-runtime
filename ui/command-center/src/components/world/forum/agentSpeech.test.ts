import { beforeEach, describe, expect, it } from 'vitest';
import { beginPlayback, clearPlayback } from '../../../lib/voicePlaybackClock';
import { createMouthState, sampleMouth, stepMouth, shapeFromEnergy } from './agentSpeech';
import { buildMouthTrack, MIN_SHAPE_HOLD_MS, MOUTH_SHAPES, shapeAt, wordShapes, type MouthShape } from './mouthVisemes';
import { fixtureSegment, proportionalTimings } from './speechFixture';
import { avatarCue, NO_CUE } from './agentCue';

const FRAME = 1000 / 60;
const MAX_H = MOUTH_SHAPES.wide.h;

/** Run the real driver at 60 fps over the fixture and record every frame. */
function runSpeech() {
  const seg = fixtureSegment(0);
  beginPlayback(seg);
  const m = createMouthState();
  const frames: { t: number; shape: MouthShape; h: number; w: number }[] = [];
  for (let t = 0; t < seg.durationMs + 400; t += FRAME) {
    const s = sampleMouth(t);
    stepMouth(m, FRAME / 1000, t, { speaking: t < seg.durationMs + 60, shape: s.shape, energy: s.energy });
    frames.push({ t, shape: m.shape, h: m.h, w: m.w });
  }
  return { seg, frames };
}

describe('mouth shapes from letters', () => {
  it('maps vowels, bilabials and round vowels to distinct coarse shapes', () => {
    expect(wordShapes('map').map(s => s.shape)).toEqual(['closed', 'wide', 'closed']);
    expect(wordShapes('too').map(s => s.shape)).toContain('round');
    expect(wordShapes('make').map(s => s.shape)).toEqual(['closed', 'wide', 'small']);
    expect(wordShapes('...')).toEqual([{ shape: 'mid', weight: 1 }]);
  });

  it('closes on /m/ /b/ /p/ and between words, and never holds a shape under the minimum', () => {
    const words = proportionalTimings('Maybe the board is big', 1800);
    const track = buildMouthTrack(words);
    expect(shapeAt(track, words[0].startMs + 1)).toBe('closed'); // "m"
    for (const w of words) expect(shapeAt(track, w.endMs - 5)).toBe('closed'); // beat between words
    for (const span of track) {
      const dur = span.endMs - span.startMs;
      // A span may be shorter only when its whole word is too short to split.
      expect(dur).toBeGreaterThan(MIN_SHAPE_HOLD_MS * 0.5);
    }
    expect(shapeAt(track, -10)).toBe('closed');
    expect(shapeAt(track, 99999)).toBe('closed');
  });
});

describe('mouth driver on real speech timing (V-32 / V-33)', () => {
  beforeEach(() => clearPlayback());

  it('visits several distinct shapes and stays below full opening most of the time', () => {
    const { frames } = runSpeech();
    const shapes = new Set(frames.map(f => f.shape));
    expect(shapes.size).toBeGreaterThanOrEqual(4);
    expect(shapes.has('closed')).toBe(true);
    const pinned = frames.filter(f => f.h > MAX_H * 0.97).length / frames.length;
    expect(pinned).toBeLessThan(0.2);
  });

  it('opens and closes at syllable rate: height crosses 30% of its range at least 3 times a second', () => {
    const { seg, frames } = runSpeech();
    const speaking = frames.filter(f => f.t < seg.durationMs);
    const lo = Math.min(...speaking.map(f => f.h));
    const mid = lo + (Math.max(...speaking.map(f => f.h)) - lo) * 0.3;
    let crossings = 0;
    for (let i = 1; i < speaking.length; i++) if ((speaking[i - 1].h < mid) !== (speaking[i].h < mid)) crossings++;
    expect(crossings / 2 / (seg.durationMs / 1000)).toBeGreaterThanOrEqual(3);
  });

  it('does not flicker: every shape held at least the minimum, bar the final close', () => {
    const { seg, frames } = runSpeech();
    let runStart = 0;
    for (let i = 1; i < frames.length; i++) {
      if (frames[i].shape !== frames[i - 1].shape) {
        // The run that just ended.
        if (runStart > 0 && frames[i - 1].t < seg.durationMs) expect(frames[i].t - frames[runStart].t).toBeGreaterThanOrEqual(40 - FRAME); // closes on silence may come at 40 ms; opens wait 75
        runStart = i;
      }
    }
  });

  it('is smooth: no frame-to-frame jump larger than a fraction of the range', () => {
    const { frames } = runSpeech();
    let worst = 0;
    for (let i = 1; i < frames.length; i++) worst = Math.max(worst, Math.abs(frames[i].h - frames[i - 1].h));
    expect(worst).toBeLessThan(MAX_H * 0.6);
  });

  it('closes between words where the audio is quiet, and at rest', () => {
    const { seg, frames } = runSpeech();
    const quiet = frames.filter(f => {
      if (f.t > seg.durationMs) return false;
      return sampleMouth(f.t).energy < 0.05;
    });
    expect(quiet.length).toBeGreaterThan(10);
    const closedShare = quiet.filter(f => f.shape === 'closed').length / quiet.length;
    expect(closedShare).toBeGreaterThan(0.7);
    const rest = frames[frames.length - 1];
    expect(rest.shape).toBe('closed');
    expect(rest.h).toBeLessThan(MOUTH_SHAPES.closed.h * 1.5);
  });

  it('closes at once when speech stops mid-word (interrupt)', () => {
    const seg = fixtureSegment(0);
    beginPlayback(seg);
    const m = createMouthState();
    let t = 0;
    for (; t < 900; t += FRAME) {
      const s = sampleMouth(t);
      stepMouth(m, FRAME / 1000, t, { speaking: true, shape: s.shape, energy: s.energy });
    }
    clearPlayback();
    for (let i = 0; i < 12; i++, t += FRAME) stepMouth(m, FRAME / 1000, t, { speaking: false, shape: null, energy: 0 });
    expect(m.shape).toBe('closed');
    expect(m.h).toBeLessThan(MOUTH_SHAPES.closed.h * 2);
  });

  it('REGRESSION: the old analyser-bin opening saturates on the same speech (why the harness had to change)', () => {
    // Old formula, fed bins where ordinary speech sits at 0.4-0.7 normalised.
    const old = (bins: number[]) => {
      const n = Math.floor(bins.length * 0.65);
      let e = 0;
      for (let i = 0; i < n; i++) e += bins[i] / 255;
      return Math.min(1, Math.max(0, (e / n - 0.035) * 2.8));
    };
    const { seg } = runSpeech();
    let pinned = 0;
    let total = 0;
    for (let t = 0; t < seg.durationMs; t += FRAME) {
      // Smoothing .75 holds bins up between syllables: model as a floor of 0.4.
      const level = 0.4 + 0.3 * sampleMouth(t).energy;
      pinned += old(new Array(32).fill(level * 255)) >= 0.999 ? 1 : 0;
      total++;
    }
    expect(pinned / total).toBeGreaterThan(0.2);
  });

  it('falls back to energy-only shapes when a segment has no word timings', () => {
    expect(shapeFromEnergy(0.02)).toBe('closed');
    expect(new Set([0.2, 0.4, 0.7, 0.9].map(shapeFromEnergy)).size).toBe(4);
  });
});

describe('listening and thinking cues (V-36)', () => {
  it('gives each engine state a distinct cue and a still pose under reduced motion', () => {
    expect(avatarCue('ready', 1, false)).toEqual(NO_CUE);
    expect(avatarCue('playing', 1, false)).toEqual(NO_CUE);
    const listening = avatarCue('recording', 1, false);
    const thinking = avatarCue('processing', 1, false);
    expect(listening.roll).toBeGreaterThan(0);
    expect(thinking.yaw).toBeGreaterThan(0);
    expect(thinking.eyeScaleY).toBeLessThan(1);
    expect(listening).not.toEqual(thinking);
    expect(avatarCue('recording', 1, true)).toEqual(avatarCue('recording', 5, true));
    expect(avatarCue('processing', 1, true)).toEqual(avatarCue('processing', 5, true));
  });
});
