/**
 * Playback clock for the agent's voice.
 *
 * The voice socket sends, before every binary PCM frame, an `audio_segment`
 * text frame carrying the segment's text, duration and per-word timings. The
 * client knows the exact PCM it is about to schedule, so it can say — to the
 * millisecond, on its own clock — which word is being heard and how loud the
 * speaker is right now. This module is that registry.
 *
 * It is deliberately framework-free (no React, no store): useVoice writes to
 * it when a segment actually starts playing, and the World avatar's mouth, the
 * barge-in echo guard and the playback acknowledgements read from it. The
 * audio itself is the source of truth for loudness, so the mouth closes
 * between words and at rest because the SAMPLES are quiet there, not because
 * a smoothed analyser bin happens to have decayed.
 */

export interface WordSpan {
  word: string;
  startMs: number;
  endMs: number;
}

export interface PlaybackSegment {
  segmentId: number;
  /** performance.now() at which this segment becomes AUDIBLE (output latency included). */
  startedAtMs: number;
  durationMs: number;
  text: string;
  words: WordSpan[];
  /** RMS per `binMs` of the segment's PCM. */
  envelope: Float32Array;
  binMs: number;
  /** Loud-speech reference RMS for this segment (p95, floored), for normalising. */
  reference: number;
}

export const ENVELOPE_BIN_MS = 10;
/** Floor for the normalising reference so near-silent audio never gets amplified into motion. */
const MIN_REFERENCE_RMS = 0.02;

/** RMS envelope of mono PCM, one value per `binMs`. */
export function buildEnvelope(
  samples: ArrayLike<number>,
  sampleRate: number,
  binMs: number = ENVELOPE_BIN_MS,
): { envelope: Float32Array; reference: number } {
  const per = Math.max(1, Math.round((sampleRate * binMs) / 1000));
  const bins = Math.ceil(samples.length / per);
  const envelope = new Float32Array(bins);
  for (let b = 0; b < bins; b++) {
    const from = b * per;
    const to = Math.min(samples.length, from + per);
    let sum = 0;
    for (let i = from; i < to; i++) sum += samples[i] * samples[i];
    envelope[b] = Math.sqrt(sum / Math.max(1, to - from));
  }
  return { envelope, reference: referenceLevel(envelope) };
}

function referenceLevel(envelope: Float32Array): number {
  if (envelope.length === 0) return MIN_REFERENCE_RMS;
  const sorted = Array.from(envelope).sort((a, b) => a - b);
  const p95 = sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.95))];
  return Math.max(MIN_REFERENCE_RMS, p95);
}

/** Build a registrable segment from the wire metadata and the PCM. */
export function makeSegment(opts: {
  segmentId: number;
  startedAtMs: number;
  text: string;
  words: WordSpan[];
  samples: ArrayLike<number>;
  sampleRate: number;
  /** Server-reported duration; falls back to the PCM length. */
  durationMs?: number;
}): PlaybackSegment {
  const { envelope, reference } = buildEnvelope(opts.samples, opts.sampleRate);
  const pcmMs = (opts.samples.length / opts.sampleRate) * 1000;
  return {
    segmentId: opts.segmentId,
    startedAtMs: opts.startedAtMs,
    durationMs: pcmMs > 0 ? pcmMs : (opts.durationMs ?? 0),
    text: opts.text,
    words: opts.words,
    envelope,
    binMs: ENVELOPE_BIN_MS,
    reference,
  };
}

/** Parse `word_timings` from an `audio_segment` frame, tolerating junk. */
export function parseWordTimings(raw: unknown): WordSpan[] {
  if (!Array.isArray(raw)) return [];
  const out: WordSpan[] = [];
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue;
    const r = item as Record<string, unknown>;
    const start = Number(r.start_ms);
    const end = Number(r.end_ms);
    if (typeof r.word !== 'string' || !Number.isFinite(start) || !Number.isFinite(end)) continue;
    out.push({ word: r.word, startMs: start, endMs: Math.max(start, end) });
  }
  return out;
}

let current: PlaybackSegment | null = null;
const listeners = new Set<() => void>();

export function beginPlayback(segment: PlaybackSegment): void {
  current = segment;
  listeners.forEach(l => l());
}

/** Clear when `segmentId` is given only if it is still the registered one. */
export function endPlayback(segmentId?: number): void {
  if (segmentId !== undefined && current && current.segmentId !== segmentId) return;
  current = null;
  listeners.forEach(l => l());
}

export function clearPlayback(): void {
  endPlayback();
}

export function onPlaybackChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function activeSegment(nowMs: number): PlaybackSegment | null {
  const seg = current;
  if (!seg) return null;
  const t = nowMs - seg.startedAtMs;
  // 60 ms of grace past the end so the final shape can resolve to closed.
  return t >= -5 && t <= seg.durationMs + 60 ? seg : null;
}

/** Raw (un-normalised) speaker RMS at `nowMs`, 0 when nothing is audible. */
export function rawPlaybackRms(nowMs: number): number {
  const seg = activeSegment(nowMs);
  if (!seg) return 0;
  return envelopeAt(seg, nowMs - seg.startedAtMs);
}

/** Envelope value (raw RMS) at `tMs` into the segment, linearly interpolated. */
export function envelopeAt(seg: PlaybackSegment, tMs: number): number {
  if (tMs < 0 || seg.envelope.length === 0) return 0;
  const pos = tMs / seg.binMs - 0.5;
  const i = Math.floor(pos);
  const frac = pos - i;
  const a = seg.envelope[Math.max(0, Math.min(seg.envelope.length - 1, i))] ?? 0;
  const b = seg.envelope[Math.max(0, Math.min(seg.envelope.length - 1, i + 1))] ?? 0;
  return a + (b - a) * frac;
}

/** Loudness 0..1 relative to the segment's own loud-speech level. */
export function normalisedEnergyAt(seg: PlaybackSegment, tMs: number): number {
  return Math.min(1, envelopeAt(seg, tMs) / seg.reference);
}

/** Rolling-peak hold, used by the speaker-route echo guard (mirrors the iPhone fix). */
export function createPeakHold(holdMs: number) {
  let peak = 0;
  let peakAt = -Infinity;
  return {
    update(nowMs: number, value: number): number {
      if (value >= peak || nowMs - peakAt > holdMs) {
        peak = value;
        peakAt = nowMs;
      }
      return nowMs - peakAt > holdMs ? 0 : peak;
    },
    current(nowMs: number): number {
      return nowMs - peakAt > holdMs ? 0 : peak;
    },
    reset() {
      peak = 0;
      peakAt = -Infinity;
    },
  };
}
