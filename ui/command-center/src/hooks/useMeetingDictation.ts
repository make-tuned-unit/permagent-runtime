/**
 * useMeetingDictation — chunked meeting recording → LOCAL transcription →
 * project note (call-notes MVP 1A).
 *
 * The chunked variant of useDictation: instead of one short clip, it records
 * continuously from THIS machine's microphone (own voice only — never the
 * other side of a call), slices the PCM into fixed-length chunks, and feeds
 * each chunk to `POST /api/dictation/transcribe` — the existing batch endpoint
 * backed by the on-device Whisper model. LOCAL ONLY by construction: the only
 * transcription call in this hook is the local endpoint (sovereignty ruling —
 * no cloud STT is wired, deliberately).
 *
 * Chunk uploads are serialized client-side (a promise chain) — the daemon's
 * transcriber is a serialized global anyway, and ordering keeps the transcript
 * in speech order. A failed chunk is NEVER silently dropped: it leaves an
 * explicit gap marker in the transcript and bumps `failedChunks`.
 *
 * On stop, the remaining audio is flushed, all pending transcriptions awaited,
 * and the joined transcript is saved as a note on the chosen project via the
 * existing notes path (`api.createProjectNote` → Brain-indexed, Librarian-
 * enriched). If the save fails the transcript is retained and `retrySave`
 * re-attempts — dictated words are never thrown away silently.
 *
 * Audio capture asks for a 16 kHz AudioContext (Whisper's native rate; ~3x
 * smaller uploads than 48 kHz) and falls back to the device default when the
 * webview refuses; the WAV header always carries the REAL context rate.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from '../lib/api';
import { emitActivity } from '../lib/emitActivity';
import { encodeWav } from './useDictation';

export type MeetingDictationState =
  | 'idle'
  | 'recording'
  | 'finishing' // stopped; flushing + transcribing the tail, then saving
  | 'error';

/** Seconds of audio per transcription chunk. Long enough for Whisper context,
 *  short enough that a meeting never nears the 25 MB upload cap (at 16 kHz,
 *  45 s ≈ 1.4 MB) and the transcript stays near-live. Exported for tests. */
export const MEETING_CHUNK_SECONDS = 45;

/** Marker left in the transcript where a chunk's transcription failed. */
export const GAP_MARKER = '[… a segment could not be transcribed …]';

/** True when enough samples are buffered to cut a chunk. Pure; tested. */
export function shouldFlushChunk(
  bufferedSamples: number,
  sampleRate: number,
  chunkSeconds: number = MEETING_CHUNK_SECONDS,
): boolean {
  return sampleRate > 0 && bufferedSamples >= sampleRate * chunkSeconds;
}

/** Join per-chunk transcripts into one note body: non-empty parts separated by
 *  a space, gap markers kept verbatim. Pure; tested. */
export function joinTranscript(parts: string[]): string {
  return parts
    .map(p => p.trim())
    .filter(p => p.length > 0)
    .join(' ')
    .trim();
}

/** Title for the saved note, e.g. "Meeting dictation — 20 Jul 2026, 14:05".
 *  Pure; tested via injection of a fixed date. */
export function meetingNoteTitle(startedAt: Date): string {
  const date = startedAt.toLocaleDateString(undefined, {
    day: 'numeric', month: 'short', year: 'numeric',
  });
  const time = startedAt.toLocaleTimeString(undefined, {
    hour: '2-digit', minute: '2-digit',
  });
  return `Meeting dictation — ${date}, ${time}`;
}

/** mm:ss (or h:mm:ss) for the recording indicator. Pure; tested. */
export function formatElapsed(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const mm = h > 0 ? String(m).padStart(2, '0') : String(m);
  return `${h > 0 ? `${h}:` : ''}${mm}:${String(sec).padStart(2, '0')}`;
}

export interface MeetingTarget {
  projectId: string;
  projectName: string;
}

export function useMeetingDictation() {
  const [state, setState] = useState<MeetingDictationState>('idle');
  const [error, setError] = useState<string | null>(null);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [failedChunks, setFailedChunks] = useState(0);
  const [target, setTarget] = useState<MeetingTarget | null>(null);

  const ctxRef = useRef<AudioContext | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const procRef = useRef<ScriptProcessorNode | null>(null);
  const bufferRef = useRef<Float32Array[]>([]);
  const bufferedRef = useRef(0);
  const rateRef = useRef(16000);
  const partsRef = useRef<string[]>([]);
  const queueRef = useRef<Promise<void>>(Promise.resolve());
  const startedAtRef = useRef<Date>(new Date());
  const timerRef = useRef<ReturnType<typeof setInterval>>();
  // Retained transcript when the note save failed (retrySave re-attempts).
  const unsavedRef = useRef<{ title: string; body: string } | null>(null);

  const teardownAudio = useCallback(() => {
    try { procRef.current?.disconnect(); } catch { /* already gone */ }
    streamRef.current?.getTracks().forEach(t => t.stop());
    const ctx = ctxRef.current;
    if (ctx && ctx.state !== 'closed') void ctx.close();
    procRef.current = null;
    ctxRef.current = null;
    streamRef.current = null;
    clearInterval(timerRef.current);
  }, []);

  useEffect(() => () => teardownAudio(), [teardownAudio]);

  /** Cut the buffered PCM into a WAV and queue it for serialized local
   *  transcription, appending its text (or a gap marker) in speech order. */
  const flushChunk = useCallback(() => {
    const chunks = bufferRef.current;
    const total = bufferedRef.current;
    bufferRef.current = [];
    bufferedRef.current = 0;
    if (total === 0) return;

    const merged = new Float32Array(total);
    let off = 0;
    for (const c of chunks) { merged.set(c, off); off += c.length; }
    const rate = rateRef.current;
    const slot = partsRef.current.length;
    partsRef.current.push(''); // reserve speech-order slot

    queueRef.current = queueRef.current.then(async () => {
      try {
        const wav = encodeWav(merged, rate);
        const { text } = await api.transcribeAudio(new Blob([wav], { type: 'audio/wav' }));
        partsRef.current[slot] = text;
      } catch {
        partsRef.current[slot] = GAP_MARKER;
        setFailedChunks(n => n + 1);
      }
    });
  }, []);

  const start = useCallback(async (nextTarget: MeetingTarget) => {
    if (state === 'recording' || state === 'finishing') return;
    setError(null);
    setFailedChunks(0);
    setElapsedSeconds(0);
    partsRef.current = [];
    queueRef.current = Promise.resolve();
    unsavedRef.current = null;
    setTarget(nextTarget);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;
      // 16 kHz keeps meeting-length uploads small; fall back to the device
      // default if the webview refuses the rate. The WAV header always carries
      // the real rate, so either way decodes correctly.
      let ctx: AudioContext;
      try { ctx = new AudioContext({ sampleRate: 16000 }); } catch { ctx = new AudioContext(); }
      ctxRef.current = ctx;
      rateRef.current = ctx.sampleRate;
      const source = ctx.createMediaStreamSource(stream);
      // ScriptProcessorNode: deprecated but universally available in the
      // desktop webview with no worklet loading — same trade-off useDictation
      // makes.
      const proc = ctx.createScriptProcessor(4096, 1, 1);
      procRef.current = proc;
      bufferRef.current = [];
      bufferedRef.current = 0;
      proc.onaudioprocess = (e: AudioProcessingEvent) => {
        const data = new Float32Array(e.inputBuffer.getChannelData(0));
        bufferRef.current.push(data);
        bufferedRef.current += data.length;
        if (shouldFlushChunk(bufferedRef.current, rateRef.current)) flushChunk();
      };
      source.connect(proc);
      proc.connect(ctx.destination);
      startedAtRef.current = new Date();
      timerRef.current = setInterval(() => {
        setElapsedSeconds(Math.floor((Date.now() - startedAtRef.current.getTime()) / 1000));
      }, 1000);
      setState('recording');
    } catch {
      setError('Microphone access was blocked');
      setState('error');
      teardownAudio();
    }
  }, [state, flushChunk, teardownAudio]);

  const saveNote = useCallback(async (projectId: string, title: string, body: string) => {
    await api.createProjectNote(projectId, { title, body });
    unsavedRef.current = null;
  }, []);

  /** Stop recording, transcribe the tail, and save the note. */
  const stop = useCallback(async (): Promise<boolean> => {
    if (state !== 'recording' || !target) return false;
    setState('finishing');
    flushChunk(); // tail
    teardownAudio();

    await queueRef.current; // all chunks transcribed (or gap-marked)

    const body = joinTranscript(partsRef.current);
    if (!body) {
      // Nothing transcribable — honest outcome, no empty note.
      setError('No speech was transcribed — no note was saved.');
      setState('error');
      return false;
    }
    const title = meetingNoteTitle(startedAtRef.current);
    unsavedRef.current = { title, body };
    try {
      await saveNote(target.projectId, title, body);
      // Usage-only signal (no transcript in the payload) — mirrors useDictation.
      emitActivity('dictation_completed', 'voice', { char_count: body.length });
      setState('idle');
      setTarget(null);
      return true;
    } catch (e) {
      setError(`Couldn't save the meeting note: ${(e as Error).message || 'request failed'}. Your transcript is kept — retry the save.`);
      setState('error');
      return false;
    }
  }, [state, target, flushChunk, teardownAudio, saveNote]);

  /** Re-attempt the note save after a failure (transcript was retained). */
  const retrySave = useCallback(async (): Promise<boolean> => {
    const unsaved = unsavedRef.current;
    if (!unsaved || !target) return false;
    setError(null);
    try {
      await saveNote(target.projectId, unsaved.title, unsaved.body);
      emitActivity('dictation_completed', 'voice', { char_count: unsaved.body.length });
      setState('idle');
      setTarget(null);
      return true;
    } catch (e) {
      setError(`Couldn't save the meeting note: ${(e as Error).message || 'request failed'}. Your transcript is kept — retry the save.`);
      setState('error');
      return false;
    }
  }, [target, saveNote]);

  /** Discard the recording (and any unsaved transcript) without saving. */
  const discard = useCallback(() => {
    teardownAudio();
    bufferRef.current = [];
    bufferedRef.current = 0;
    partsRef.current = [];
    unsavedRef.current = null;
    setTarget(null);
    setError(null);
    setState('idle');
  }, [teardownAudio]);

  return {
    state,
    error,
    elapsedSeconds,
    failedChunks,
    target,
    /** True when a failed save left a transcript to retry. */
    hasUnsavedTranscript: unsavedRef.current !== null,
    start,
    stop,
    retrySave,
    discard,
  };
}
