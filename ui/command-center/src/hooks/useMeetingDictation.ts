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

/**
 * Compose the two-sided meeting transcript as markdown.
 *
 * The microphone track is the user; the ScreenCaptureKit track is everyone
 * else on the call. Both are cut on the SAME chunk cadence, so index `i` of
 * each covers the same wall-clock window — that is what lets the two be
 * interleaved into something that reads chronologically instead of two
 * monologues stapled together.
 *
 * Speaker attribution is per-SIDE, not per-person: system audio is one mixed
 * stream, so "Others" is the honest label. Claiming individual names here
 * would be diarisation we have not done, and a transcript that invents who
 * said what is worse than one that says "Others".
 */
export function composeMeetingTranscript(mine: string[], theirs: string[]): string {
  const rounds = Math.max(mine.length, theirs.length);
  const out: string[] = [];
  for (let i = 0; i < rounds; i++) {
    const me = (mine[i] ?? '').trim();
    const them = (theirs[i] ?? '').trim();
    if (them) out.push(`**Others:** ${them}`);
    if (me) out.push(`**You:** ${me}`);
  }
  return out.join('\n\n').trim();
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

/** Where the in-flight transcript is stashed so an app quit/crash mid-meeting
 *  cannot destroy it (2026-08-06: an install mid-call ate half a meeting —
 *  the module's own contract says words are never dropped). Updated after
 *  every transcribed chunk; cleared on save or discard. */
const DRAFT_KEY = 'permagent-meeting-draft';

interface MeetingDraft {
  projectId: string;
  projectName: string;
  startedAt: string; // ISO
  parts: string[];
  farParts: string[];
  /** What the user typed in the notepad while recording. */
  userNotes?: string;
}

/** Assemble the saved body: the user's own notes (verbatim, in their own
 *  section) ahead of the transcript. The daemon's enhancement pass splits on
 *  these exact headings, and the structure IS the provenance — a reader can
 *  always tell the user's words from the machine's. */
export function composeMeetingBody(transcript: string, userNotes: string): string {
  const notes = userNotes.trim();
  if (!notes) return transcript;
  return `## Your notes\n\n${notes}\n\n## Transcript\n\n${transcript}`;
}

/** A draft's full body, notes included — used by the recovery path. */
function draftBody(d: MeetingDraft): string {
  return composeMeetingBody(composeDraftBody(d), d.userNotes ?? '');
}

function readDraft(): MeetingDraft | null {
  try {
    const raw = localStorage.getItem(DRAFT_KEY);
    if (!raw) return null;
    const d = JSON.parse(raw) as MeetingDraft;
    if (!d.projectId || !Array.isArray(d.parts)) return null;
    return d;
  } catch { return null; }
}

/** Compose a draft's body with the same honesty rule as a live stop: the
 *  two-speaker form only when far-side audio actually exists. */
export function composeDraftBody(d: MeetingDraft): string {
  const anyFar = d.farParts.some(p => p.trim().length > 0);
  return anyFar ? composeMeetingTranscript(d.parts, d.farParts) : joinTranscript(d.parts);
}

export function useMeetingDictation() {
  const [state, setState] = useState<MeetingDictationState>('idle');
  const [error, setError] = useState<string | null>(null);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [failedChunks, setFailedChunks] = useState(0);
  const [target, setTarget] = useState<MeetingTarget | null>(null);
  // Far-side capture (ScreenCaptureKit) — the OTHER participants, which the
  // microphone cannot hear. Separate slot list from the mic so the two can be
  // interleaved and labelled; `farQueueRef` serialises its transcription for
  // the same reason the mic queue does (one local Whisper, ordering matters).
  const [systemAudio, setSystemAudio] = useState(false);
  const [systemAudioError, setSystemAudioError] = useState<string | null>(null);
  // Proof-of-capture: how many far-side chunks have actually arrived. The
  // indicator uses this to say "hearing the call" instead of leaving the user
  // guessing whether the other participants are really being recorded.
  const [farChunksHeard, setFarChunksHeard] = useState(0);
  const farPartsRef = useRef<string[]>([]);
  const farQueueRef = useRef<Promise<void>>(Promise.resolve());
  const unlistenRef = useRef<Array<() => void>>([]);

  // A transcript stranded by a crash/quit, found at mount. Non-null until the
  // user saves or discards it.
  const [recoveredDraft, setRecoveredDraft] = useState<MeetingDraft | null>(() => {
    const d = readDraft();
    return d && composeDraftBody(d).length > 0 ? d : null;
  });

  /** The notepad: what the user types while the meeting runs. Sparse by
   *  design — these fragments steer what the summary argues, they are not
   *  bookmarks. Stashed with every chunk so a crash cannot lose them. */
  const [userNotes, setUserNotes] = useState('');
  const userNotesRef = useRef('');
  userNotesRef.current = userNotes;

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

  /** Persist the in-flight transcript. Called after every chunk lands so the
   *  stash is never more than one chunk behind reality. */
  const targetRef = useRef<MeetingTarget | null>(null);
  const stashDraft = useCallback(() => {
    const t = targetRef.current;
    if (!t) return;
    try {
      localStorage.setItem(DRAFT_KEY, JSON.stringify({
        projectId: t.projectId,
        projectName: t.projectName,
        startedAt: startedAtRef.current.toISOString(),
        parts: partsRef.current,
        farParts: farPartsRef.current,
        userNotes: userNotesRef.current,
      } satisfies MeetingDraft));
    } catch { /* quota/serialization — the live path is unaffected */ }
  }, []);
  const clearDraft = useCallback(() => {
    try { localStorage.removeItem(DRAFT_KEY); } catch { /* nothing to clear */ }
  }, []);


  /** Is the ScreenCaptureKit sidecar present in this build? */
  const systemAudioAvailable = useCallback(async (): Promise<boolean> => {
    if (!('__TAURI_INTERNALS__' in window)) return false;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      return await invoke<boolean>('system_audio_available');
    } catch { return false; }
  }, []);

  /** Start far-side capture and stream its chunks into the transcript.
   *  Failure here is NON-FATAL: the mic keeps recording and the meeting is
   *  captured one-sided rather than not at all. */
  const startSystemAudio = useCallback(async () => {
    if (!('__TAURI_INTERNALS__' in window)) return;
    setSystemAudioError(null);
    setFarChunksHeard(0);
    farPartsRef.current = [];
    farQueueRef.current = Promise.resolve();
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const { listen } = await import('@tauri-apps/api/event');

      const offChunk = await listen<string>('system_audio_chunk', e => {
        const path = e.payload;
        const slot = farPartsRef.current.length;
        farPartsRef.current.push('');
        setFarChunksHeard(n => n + 1);
        farQueueRef.current = farQueueRef.current.then(async () => {
          try {
            const bytes = await invoke<number[]>('read_audio_chunk', { path });
            const blob = new Blob([new Uint8Array(bytes)], { type: 'audio/wav' });
            const { text } = await api.transcribeAudio(blob);
            farPartsRef.current[slot] = text;
          } catch {
            farPartsRef.current[slot] = GAP_MARKER;
            setFailedChunks(n => n + 1);
          }
          stashDraft();
        });
      });
      const offErr = await listen<{ kind: string; detail: string }>('system_audio_error', e => {
        // "permission" is the user's to resolve and must say so plainly;
        // anything else is ours. Conflating them is what makes this
        // undebuggable from a screenshot.
        setSystemAudioError(
          e.payload.kind === 'permission'
            ? 'Screen Recording permission is off, so the other participants are not being recorded. Grant it to Permagent in System Settings → Privacy & Security → Screen Recording, then restart the recording.'
            : `System audio capture failed (${e.payload.detail}). Your own voice is still being recorded.`,
        );
      });
      unlistenRef.current = [offChunk, offErr];

      const outDir = `/tmp/permagent-audiocap-${Date.now()}`;
      await invoke('start_system_audio', { outDir, chunkSeconds: MEETING_CHUNK_SECONDS });
    } catch (e) {
      setSystemAudioError(`Couldn't start system audio capture: ${(e as Error).message ?? e}. Your own voice is still being recorded.`);
    }
  }, [stashDraft]);

  const stopSystemAudio = useCallback(async () => {
    unlistenRef.current.forEach(fn => { try { fn(); } catch { /* already gone */ } });
    unlistenRef.current = [];
    if (!('__TAURI_INTERNALS__' in window)) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('stop_system_audio');
    } catch { /* helper already gone; the flush is best-effort by design */ }
  }, []);

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
      stashDraft();
    });
  }, [stashDraft]);

  const start = useCallback(async (nextTarget: MeetingTarget) => {
    if (state === 'recording' || state === 'finishing') return;
    setError(null);
    setFailedChunks(0);
    setElapsedSeconds(0);
    partsRef.current = [];
    queueRef.current = Promise.resolve();
    setUserNotes('');
    userNotesRef.current = '';
    if (systemAudio) await startSystemAudio();
    unsavedRef.current = null;
    setTarget(nextTarget);
    targetRef.current = nextTarget;
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
      // Seed the stash immediately: even a crash before the first chunk
      // leaves a record of which project the recording was for.
      stashDraft();
      timerRef.current = setInterval(() => {
        setElapsedSeconds(Math.floor((Date.now() - startedAtRef.current.getTime()) / 1000));
      }, 1000);
      setState('recording');
    } catch {
      setError('Microphone access was blocked');
      setState('error');
      teardownAudio();
    }
  }, [state, flushChunk, teardownAudio, startSystemAudio, systemAudio, stashDraft]);

  const saveNote = useCallback(async (projectId: string, title: string, body: string) => {
    // kind:'meeting' triggers the daemon's background action-item extraction —
    // to-dos stated in the meeting land on the project's kanban unasked.
    await api.createProjectNote(projectId, { title, body, kind: 'meeting' });
    unsavedRef.current = null;
    clearDraft();
  }, [clearDraft]);

  /** Stop recording, transcribe the tail, and save the note. */
  const stop = useCallback(async (): Promise<boolean> => {
    if (state !== 'recording' || !target) return false;
    setState('finishing');
    flushChunk(); // tail
    teardownAudio();

    await stopSystemAudio();
    await queueRef.current;      // mic chunks transcribed (or gap-marked)
    await farQueueRef.current;   // far-side chunks likewise

    // With far-side audio the transcript is a two-speaker markdown document;
    // without it, the original single-voice paragraph. Not always the labelled
    // form: labelling a mic-only recording "You:" throughout implies the other
    // half was captured and was silent, which is a lie about coverage.
    const anyFar = farPartsRef.current.some(p => p.trim().length > 0);
    const body = anyFar
      ? composeMeetingTranscript(partsRef.current, farPartsRef.current)
      : joinTranscript(partsRef.current);
    if (!body) {
      // Nothing transcribable — honest outcome, no empty note.
      setError('No speech was transcribed — no note was saved.');
      setState('error');
      return false;
    }
    const title = meetingNoteTitle(startedAtRef.current);
    const composed = composeMeetingBody(body, userNotesRef.current);
    unsavedRef.current = { title, body: composed };
    try {
      await saveNote(target.projectId, title, composed);
      // Usage-only signal (no transcript in the payload) — mirrors useDictation.
      emitActivity('dictation_completed', 'voice', { char_count: composed.length });
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
    setUserNotes('');
    clearDraft();
    setTarget(null);
    setError(null);
    setState('idle');
  }, [teardownAudio, clearDraft]);

  /** Save a crash-recovered transcript as the meeting note it should have
   *  been. Same path as a live save, so the kanban extraction runs too. */
  const recoverDraft = useCallback(async (): Promise<boolean> => {
    const d = recoveredDraft;
    if (!d) return false;
    const body = draftBody(d);
    const title = `${meetingNoteTitle(new Date(d.startedAt))} (recovered)`;
    try {
      await api.createProjectNote(d.projectId, { title, body, kind: 'meeting' });
      emitActivity('dictation_completed', 'voice', { char_count: body.length });
      // Only clear the stored slot if it still holds THIS draft — a new
      // recording may have claimed it since.
      const stored = readDraft();
      if (stored && stored.startedAt === d.startedAt) clearDraft();
      setRecoveredDraft(null);
      return true;
    } catch (e) {
      setError(`Couldn't save the recovered transcript: ${(e as Error).message || 'request failed'}.`);
      return false;
    }
  }, [recoveredDraft, clearDraft]);

  /** Let the recovered transcript go. */
  const dismissDraft = useCallback(() => {
    const stored = readDraft();
    if (stored && recoveredDraft && stored.startedAt === recoveredDraft.startedAt) clearDraft();
    setRecoveredDraft(null);
  }, [recoveredDraft, clearDraft]);

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
    /** Far-side (system audio) capture — the other participants. */
    systemAudio,
    setSystemAudio,
    systemAudioError,
    systemAudioAvailable,
    /** Far-side chunks actually received this recording — proof of capture. */
    farChunksHeard,
    /** The notepad the user types into while recording. */
    userNotes,
    setUserNotes,
    /** A transcript stranded by a crash/quit, awaiting save or dismissal. */
    recoveredDraft,
    recoverDraft,
    dismissDraft,
  };
}
