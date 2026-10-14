/**
 * Speech-shaped test material for the mouth: PCM whose loudness follows
 * syllables and dips between words, plus the proportional word timings the
 * server sends in `audio_segment` (voice.rs estimate_word_timings). Shared by
 * the unit tests and the manual review harness so both exercise real speech
 * timing instead of an on/off tone.
 */
import { makeSegment, type PlaybackSegment, type WordSpan } from '../../../lib/voicePlaybackClock';

export const FIXTURE_TEXT = 'Hello there, I can help you check the garden and the board today.';
export const FIXTURE_RATE = 24000;

/** Same arithmetic as the daemon: time proportional to each word's character count. */
export function proportionalTimings(text: string, durationMs: number): WordSpan[] {
  const words = text.split(/\s+/).filter(Boolean);
  const total = words.reduce((n, w) => n + w.length, 0);
  let acc = 0;
  return words.map(word => {
    const startMs = (durationMs * acc) / total;
    acc += word.length;
    return { word, startMs, endMs: (durationMs * acc) / total };
  });
}

export function speechPcm(text: string = FIXTURE_TEXT, msPerChar: number = 62): { samples: Float32Array; words: WordSpan[]; durationMs: number } {
  const durationMs = text.replace(/\s+/g, '').length * msPerChar;
  const words = proportionalTimings(text, durationMs);
  const n = Math.floor((durationMs / 1000) * FIXTURE_RATE);
  const samples = new Float32Array(n);
  for (const w of words) {
    const len = w.endMs - w.startMs;
    for (let i = Math.floor((w.startMs / 1000) * FIXTURE_RATE); i < Math.min(n, Math.floor((w.endMs / 1000) * FIXTURE_RATE)); i++) {
      const tw = (i / FIXTURE_RATE) * 1000 - w.startMs;
      const phase = tw / len;
      // Syllable pulses (~4.5 Hz) under a word envelope that dies in the last 22%.
      const syll = 0.35 + 0.65 * Math.abs(Math.sin(Math.PI * (tw / 1000) * 4.5));
      const word = phase > 0.78 ? Math.max(0, 1 - (phase - 0.78) / 0.12) : Math.min(1, phase / 0.08);
      const carrier = Math.sin((2 * Math.PI * 140 * i) / FIXTURE_RATE) + 0.4 * Math.sin((2 * Math.PI * 420 * i) / FIXTURE_RATE);
      samples[i] = 0.25 * syll * word * carrier;
    }
  }
  return { samples, words, durationMs };
}

export function fixtureSegment(startedAtMs = 0, text: string = FIXTURE_TEXT): PlaybackSegment {
  const { samples, words, durationMs } = speechPcm(text);
  return makeSegment({ segmentId: 0, startedAtMs, text, words, samples, sampleRate: FIXTURE_RATE, durationMs });
}
