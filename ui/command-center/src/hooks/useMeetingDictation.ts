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
import { api, type MeetingTriageReceipt, type MeetingTriageStatus } from '../lib/api';
import { emitActivity } from '../lib/emitActivity';
import {
  listSystemAudioApps,
  listenSystemAudio,
  type SystemAudioApp,
  readSystemAudioChunk,
  startSystemAudioCapture,
  stopSystemAudioCapture,
  systemAudioAvailable as probeSystemAudio,
} from '../lib/systemAudio';
import { encodeWav } from './useDictation';

export type MeetingDictationState =
  | 'idle'
  | 'recording'
  | 'finishing' // stopped; flushing + transcribing the tail, then saving
  | 'error';

/** Identity of a just-saved meeting note, for deep-linking from the
 *  "note saved" notification straight to the note in Projects. */
export interface SavedNoteRef {
  projectId: string;
  noteId: string;
}

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

/** The sources one recording captures. */
export interface RecordingSources {
  mic: boolean;
  system: boolean;
  /** The single app captured, or null for all system audio. */
  app: SystemAudioApp | null;
}

/** Human label for what is being captured — the actual sources, so a
 *  system-only recording never says "your voice". Pure; tested. */
export function describeSources(s: RecordingSources): string {
  const far = s.app ? `${s.app.name} audio` : 'all system audio';
  if (s.mic && s.system) return `Your microphone + ${far}`;
  if (s.system) return s.app ? `${s.app.name} audio only` : 'All system audio only';
  return 'Your microphone';
}

/** Whether a chosen app can be captured right now, given the latest app list
 *  (`undefined` = still asking, `null` = this native cannot target apps).
 *  Pure; tested. */
export function appAvailability(
  app: SystemAudioApp,
  running: SystemAudioApp[] | null | undefined,
): 'available' | 'checking' | 'unavailable' {
  if (running === undefined) return 'checking';
  return running?.some(a => a.bundleId === app.bundleId) ? 'available' : 'unavailable';
}

/** Why a chosen app cannot be recorded. Pure; tested. */
export function unavailableAppMessage(app: SystemAudioApp, running: SystemAudioApp[] | null | undefined): string {
  return running === null
    ? `This build can't capture a single app, so “${app.name}” can't be recorded on its own. Choose “All system audio” or turn system audio off.`
    : `“${app.name}” isn't running, so it can't be recorded. Open it, or choose another source.`;
}

export interface MeetingTarget {
  projectId: string;
  projectName: string;
}

/** Where in-flight transcripts are stashed so an app quit/crash mid-meeting
 *  cannot destroy them (2026-08-06: an install mid-call ate half a meeting —
 *  the module's own contract says words are never dropped). Updated after
 *  every transcribed chunk; cleared on save or discard.
 *
 *  One slot PER RECORDING, keyed by start time. A single shared slot meant the
 *  next recording silently overwrote a draft the user had not recovered yet —
 *  destroying the transcript this stash exists to protect, in the one case
 *  where it mattered. */
const DRAFT_PREFIX = 'permagent-meeting-draft:';
/** The pre-2026-08-07 single slot, migrated on first read. */
const LEGACY_DRAFT_KEY = 'permagent-meeting-draft';
/** Stranded drafts kept before the oldest are pruned. Bounded because this is
 *  localStorage: an unbounded stash of meeting transcripts eventually hits the
 *  quota and then NOTHING can be stashed, which is the failure this guards. */
export const MAX_DRAFTS = 5;
/** Quiet time after a keystroke before the notepad is stashed. Exported for tests. */
export const NOTES_STASH_DEBOUNCE_MS = 400;

export function draftKey(startedAtISO: string): string {
  return `${DRAFT_PREFIX}${startedAtISO}`;
}

export interface MeetingDraft {
  projectId: string;
  projectName: string;
  startedAt: string; // ISO
  parts: string[];
  farParts: string[];
  /** What the user typed in the notepad while recording. */
  userNotes?: string;
  /** The recording's stable identity, minted at Start. The daemon's meeting
   *  save is idempotent on it, so it must survive failure, retry and
   *  recovery unchanged. Absent on drafts written before it existed. */
  recordingId?: string;
  /** The exact title/body the stop composed, frozen once the recording ended:
   *  recovering re-sends these bytes, so a save whose response was lost is
   *  recognised as the same note rather than a conflicting one. */
  title?: string;
  body?: string;
}

/** A fresh recording identity. */
export function newRecordingId(): string {
  const c = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto;
  if (c?.randomUUID) return c.randomUUID();
  return `rec-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

/** The identity a draft is saved under. A legacy draft derives one from its
 *  start time — deterministic, so two recovery attempts are still one save.
 *  Pure; tested. */
export function draftRecordingId(d: MeetingDraft): string {
  return d.recordingId ?? `legacy-${d.startedAt}`;
}

// ── Orchestrator triage of a saved meeting ──────────────────────────────────

/** Statuses after which polling stops. */
export function isTriageSettled(status: MeetingTriageStatus): boolean {
  return status === 'completed' || status === 'failed' || status === 'skipped';
}

/** Backoff between triage polls; the last value repeats. Exported for tests. */
export const TRIAGE_POLL_DELAYS_MS = [2000, 3000, 5000, 8000, 13000, 20000, 30000];
/** Polls before giving up and offering a manual check (~15 min). */
export const MAX_TRIAGE_POLLS = 40;
/** Consecutive failed polls before giving up. */
export const MAX_TRIAGE_POLL_ERRORS = 3;

export function triagePollDelay(attempt: number): number {
  return TRIAGE_POLL_DELAYS_MS[Math.min(attempt, TRIAGE_POLL_DELAYS_MS.length - 1)];
}

/** The meeting most recently saved, so its triage can be followed again
 *  after a reload. One slot: it is the "what happened to my last call" card. */
export interface LastSavedMeeting {
  projectId: string;
  projectName: string;
  recordingId: string;
  noteId: string;
  savedAt: string; // ISO
}

const LAST_SAVED_KEY = 'permagent-meeting-last-saved';
/** A receipt older than this is history, not something to follow up. */
export const LAST_SAVED_TTL_MS = 24 * 60 * 60 * 1000;

export function writeLastSaved(m: LastSavedMeeting): void {
  try { localStorage.setItem(LAST_SAVED_KEY, JSON.stringify(m)); } catch { /* storage unavailable */ }
}

export function clearLastSaved(): void {
  try { localStorage.removeItem(LAST_SAVED_KEY); } catch { /* storage unavailable */ }
}

/** The last saved meeting, if recent and well-formed. Exported for tests. */
export function readLastSaved(now: number = Date.now()): LastSavedMeeting | null {
  try {
    const raw = localStorage.getItem(LAST_SAVED_KEY);
    if (!raw) return null;
    const m = JSON.parse(raw) as LastSavedMeeting;
    if (!m.projectId || !m.recordingId || !m.noteId || !m.savedAt) return null;
    if (now - Date.parse(m.savedAt) > LAST_SAVED_TTL_MS) return null;
    return m;
  } catch { return null; }
}

/** What the recorder knows about the last saved meeting's triage. The note
 *  being saved and the triage being done are separate facts, kept apart. */
export interface MeetingTriageView {
  meeting: LastSavedMeeting;
  /** null until the daemon has answered — or when it predates triage. */
  receipt: MeetingTriageReceipt | null;
  /** The daemon returned no receipt (older daemon) or has no row (404). */
  unavailable: boolean;
  /** Polling stopped without a settled status; a manual check is offered. */
  stalled: boolean;
  error: string | null;
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

/** Stands in the transcript section when the user typed notes but no speech
 *  was transcribed — says so rather than leaving a heading over nothing. */
export const NO_SPEECH_MARKER = '[no speech was transcribed]';

/**
 * The note body for one recording, or '' when there is nothing to save.
 *
 * Notes typed during a silent (or dead-mic) recording are still the user's
 * words: refusing to save them because the machine heard nothing threw them
 * away. Shared by the live stop and the recovery path. Pure; tested.
 */
export function meetingNoteBody(transcript: string, userNotes: string): string {
  if (!transcript && !userNotes.trim()) return '';
  return composeMeetingBody(transcript || NO_SPEECH_MARKER, userNotes);
}

/** The user-facing reason a microphone could not be started. Only a refusal
 *  is "blocked" — calling a missing or busy device "blocked" sends the user to
 *  a permission setting that is already on. */
export function micStartError(e: unknown): string {
  const name = (e as { name?: string } | null)?.name;
  if (name === 'NotAllowedError' || name === 'SecurityError') return 'Microphone access was blocked';
  if (name === 'NotFoundError') return 'No microphone was found';
  const detail = (e as Error | null)?.message;
  return `Couldn't start the microphone${detail ? ` (${detail})` : ''}`;
}

/**
 * Far-side chunks, stamped with the recording they belong to.
 *
 * A bare array outlives its recording: record a call WITH system audio, stop,
 * then record one WITHOUT it, and the second note is composed two-sided from
 * the first call's words. Stamping makes that structurally impossible rather
 * than dependent on somebody remembering to clear a ref — [`farPartsFor`]
 * returns nothing whenever the stamp does not match the live recording.
 */
export interface FarSideStash {
  recordingId: number;
  parts: string[];
  /** Chunk paths already taken. A path arrives as an event AND, from newer
   *  natives, in the list `stop_system_audio` returns; each is used once. */
  seen?: Set<string>;
}

/** The far-side parts that belong to `recordingId`, or none. Pure; tested. */
export function farPartsFor(recordingId: number, stash: FarSideStash): string[] {
  return stash.recordingId === recordingId ? stash.parts : [];
}

/** A draft's full body, notes included — used by the recovery path. */
function draftBody(d: MeetingDraft): string {
  return d.body ?? meetingNoteBody(composeDraftBody(d), d.userNotes ?? '');
}

function parseDraft(raw: string | null): MeetingDraft | null {
  if (!raw) return null;
  try {
    const d = JSON.parse(raw) as MeetingDraft;
    if (!d.projectId || !d.startedAt || !Array.isArray(d.parts)) return null;
    if (!Array.isArray(d.farParts)) d.farParts = [];
    return d;
  } catch { return null; }
}

/** Move the old single-slot draft into a keyed one. Runs once, at mount. */
export function migrateLegacyDraft(): void {
  try {
    const raw = localStorage.getItem(LEGACY_DRAFT_KEY);
    if (!raw) return;
    const d = parseDraft(raw);
    if (d) localStorage.setItem(draftKey(d.startedAt), raw);
    localStorage.removeItem(LEGACY_DRAFT_KEY);
  } catch { /* storage unavailable — nothing to migrate */ }
}

/** Every stashed draft, newest first. Exported for tests. */
export function readDrafts(): MeetingDraft[] {
  const found: MeetingDraft[] = [];
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key?.startsWith(DRAFT_PREFIX)) continue;
      const d = parseDraft(localStorage.getItem(key));
      if (d) found.push(d);
    }
  } catch { /* storage unavailable */ }
  return found.sort((a, b) => b.startedAt.localeCompare(a.startedAt));
}

/** Drop the oldest drafts beyond `MAX_DRAFTS`, never the one being written.
 *  Exported for tests. */
export function pruneDrafts(keepStartedAt: string): void {
  const doomed = readDrafts()
    .filter(d => d.startedAt !== keepStartedAt)
    .slice(MAX_DRAFTS - 1);
  try {
    doomed.forEach(d => localStorage.removeItem(draftKey(d.startedAt)));
  } catch { /* storage unavailable */ }
}

/** Compose a draft's body with the same honesty rule as a live stop: the
 *  two-speaker form only when far-side audio actually exists. */
export function composeDraftBody(d: MeetingDraft): string {
  return composeTranscriptBody(d.parts, d.farParts);
}

/**
 * The transcript body for one recording.
 *
 * Two-speaker markdown ONLY when the far side actually produced words: writing
 * "**You:**" over every line of a mic-only recording implies the other half was
 * captured and happened to be silent, which is a lie about coverage. The live
 * stop path and the crash-recovery path share this so a recovered transcript
 * can never read differently from the one that was never interrupted.
 */
export function composeTranscriptBody(parts: string[], farParts: string[]): string {
  const anyFar = farParts.some(p => p.trim().length > 0);
  return anyFar ? composeMeetingTranscript(parts, farParts) : joinTranscript(parts);
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
  /** Record this machine's microphone. On by default — the original
   *  behaviour; off makes a system-audio-only recording (no mic is opened). */
  const [recordMic, setRecordMic] = useState(true);
  /** The one app whose audio to capture, or null for all system audio (the
   *  original behaviour). Only meaningful with `systemAudio` on. */
  const [systemAudioApp, setSystemAudioApp] = useState<SystemAudioApp | null>(null);
  /** What the recording in hand is actually capturing, fixed at Start — the
   *  panel labels come from this, never from the (editable) picker toggles. */
  const [activeSources, setActiveSources] = useState<RecordingSources | null>(null);
  // Count only far-side chunks transcribed to words. An arriving silent WAV
  // or failed transcription must not light the panel's "hearing" indicator.
  const [farChunksHeard, setFarChunksHeard] = useState(0);
  const farStashRef = useRef<FarSideStash>({ recordingId: 0, parts: [] });
  const farQueueRef = useRef<Promise<void>>(Promise.resolve());
  const unlistenRef = useRef<Array<() => void>>([]);
  /** Identity of the current recording. Bumped by `start`, and the stamp that
   *  keeps one meeting's far-side audio out of the next one's note. */
  const recordingIdRef = useRef(0);
  // Near-side proof-of-capture: mic chunks that transcribed to actual words.
  // Without it a dead microphone looks exactly like a quiet one for the whole
  // meeting and only announces itself as "No speech was transcribed" at stop —
  // the one moment it is too late to do anything about it.
  const [nearChunksHeard, setNearChunksHeard] = useState(0);

  /** The far-side parts of the recording currently in hand. */
  const liveFarParts = useCallback(
    () => farPartsFor(recordingIdRef.current, farStashRef.current),
    [],
  );

  // Transcripts stranded by a crash/quit, found at mount, newest first. Each
  // stays until the user saves or discards it.
  // A notes-only draft counts: the notepad is the user's words too.
  const [recoveredDrafts, setRecoveredDrafts] = useState<MeetingDraft[]>(() => {
    migrateLegacyDraft();
    return readDrafts().filter(d => draftBody(d).length > 0);
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
  const unsavedRef = useRef<{ title: string; body: string; recordingId: string } | null>(null);
  // In-flight guards. `state` is a render-time snapshot, so two clicks inside
  // one frame both saw 'recording' (or 'error') and each saved a note — two
  // copies of the meeting, and the kanban pass ran twice.
  const startingRef = useRef(false);
  const stoppingRef = useRef(false);
  const savingRef = useRef(false);

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

  /** Persist the in-flight transcript. Called after every chunk lands so the
   *  stash is never more than one chunk behind reality. */
  const targetRef = useRef<MeetingTarget | null>(null);
  /** The live recording's stable identity (see `MeetingDraft.recordingId`). */
  const meetingIdRef = useRef('');
  const stashDraft = useCallback((final?: { title: string; body: string }) => {
    const t = targetRef.current;
    if (!t) return;
    const startedAt = startedAtRef.current.toISOString();
    try {
      localStorage.setItem(draftKey(startedAt), JSON.stringify({
        projectId: t.projectId,
        projectName: t.projectName,
        startedAt,
        parts: partsRef.current,
        farParts: liveFarParts(),
        userNotes: userNotesRef.current,
        recordingId: meetingIdRef.current,
        ...final,
      } satisfies MeetingDraft));
      pruneDrafts(startedAt);
    } catch { /* quota/serialization — the live path is unaffected */ }
  }, []);
  const clearDraft = useCallback((startedAt: string) => {
    try { localStorage.removeItem(draftKey(startedAt)); } catch { /* nothing to clear */ }
  }, []);

  // The panel promises the notepad is "saved as you type". Chunks only land
  // every 45 s, so without this a crash lost up to 45 s of typing.
  useEffect(() => {
    if (state !== 'recording') return;
    const t = setTimeout(() => stashDraft(), NOTES_STASH_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [userNotes, state, stashDraft]);


  /** Is the ScreenCaptureKit sidecar present in this build? */
  const systemAudioAvailable = useCallback(() => probeSystemAudio(), []);

  /** The current recording's far-side chunk intake. Events and the paths
   *  `stop_system_audio` returns both go through it, so a path is only ever
   *  transcribed once. Null between recordings. */
  const farIntakeRef = useRef<((path: string) => void) | null>(null);
  /** Ends a system-audio-only recording whose helper failed after Start.
   *  Assigned below, once the stop machinery it needs exists. */
  const systemOnlyFailRef = useRef<((recordingId: number, message: string) => Promise<void>) | null>(null);
  /** Set when a failed recording's draft was handed to the recovery card, so
   *  dismissing the error panel does not delete it. Cleared by Start. */
  const draftKeptRef = useRef(false);

  /** Start far-side capture and stream its chunks into the transcript.
   *  Resolves to null on success or the failure message. With the mic live a
   *  failure is NON-FATAL — it is shown on the panel and the meeting is
   *  captured one-sided rather than not at all; without the mic the caller
   *  treats it as the start failing, because nothing would be recording. */
  const startSystemAudio = useCallback(async (
    recordingId: number,
    opts: { micLive: boolean; appBundleId: string | null },
  ): Promise<string | null> => {
    const stillRecording = opts.micLive ? ' Your microphone is still being recorded.' : '';
    if (!('__TAURI_INTERNALS__' in window)) {
      return 'System audio capture is only available in the desktop app.';
    }
    const stash: FarSideStash = { recordingId, parts: [], seen: new Set() };
    farStashRef.current = stash;
    farQueueRef.current = Promise.resolve();
    // One intake per recording, closed over ITS stash: a stale callback from
    // an earlier recording can only ever write to that recording's stash, and
    // the live-id check stops even that.
    const intake = (path: string) => {
      // A chunk arriving after this recording ended belongs to nothing. The
      // live id is checked too: a mic-only recording never re-stamps the
      // stash, so the stamp alone still matched the previous call.
      if (farStashRef.current !== stash || recordingIdRef.current !== recordingId) return;
      if (stash.seen!.has(path)) return;
      stash.seen!.add(path);
      const slot = stash.parts.length;
      stash.parts.push('');
      const isCurrent = () => farStashRef.current === stash && recordingIdRef.current === recordingId;
      farQueueRef.current = farQueueRef.current.then(async () => {
        if (!isCurrent()) return;
        try {
          const bytes = await readSystemAudioChunk(path);
          const blob = new Blob([bytes], { type: 'audio/wav' });
          const { text } = await api.transcribeAudio(blob);
          if (!isCurrent()) return;
          stash.parts[slot] = text;
          if (text.trim().length > 0) setFarChunksHeard(n => n + 1);
        } catch {
          if (!isCurrent()) return;
          stash.parts[slot] = GAP_MARKER;
          setFailedChunks(n => n + 1);
        }
        stashDraft();
      });
    };
    try {
      // The helper is owned by the app process, not this webview: a reload
      // mid-call leaves it running, and `start_system_audio` then returns OK
      // without starting anything — this recording would inherit a capture
      // whose chunks were cut on someone else's clock. Stop it first. Its
      // returned paths belong to that orphaned capture and are not taken.
      try { await stopSystemAudioCapture(); } catch { /* nothing was running */ }

      const off = await listenSystemAudio(intake, payload => {
        if (recordingIdRef.current !== recordingId) return;
        // "permission" is the user's to resolve and must say so plainly;
        // anything else is ours. Conflating them is what makes this
        // undebuggable from a screenshot.
        if (!opts.micLive) {
          // System audio was the ONLY source: carrying on would be a
          // "Recording" indicator over nothing. End the recording instead.
          void systemOnlyFailRef.current?.(recordingId, payload.kind === 'permission'
            ? 'Screen Recording permission is off, and system audio was the only source, so recording stopped. Grant it to Permagent in System Settings → Privacy & Security → Screen Recording, then start again.'
            : `System audio capture failed (${payload.detail}), and it was the only source, so recording stopped.`);
          return;
        }
        setSystemAudioError(
          payload.kind === 'permission'
            ? 'Screen Recording permission is off, so the other participants are not being recorded. Grant it to Permagent in System Settings → Privacy & Security → Screen Recording, then restart the recording.'
            : `System audio capture failed (${payload.detail}).${stillRecording}`,
        );
      });
      unlistenRef.current = [off];
      farIntakeRef.current = intake;

      const outDir = `/tmp/permagent-audiocap-${Date.now()}`;
      await startSystemAudioCapture(outDir, MEETING_CHUNK_SECONDS, opts.appBundleId);
      return null;
    } catch (e) {
      farIntakeRef.current = null;
      // Listeners registered before the start failed would otherwise outlive
      // it and keep accepting chunks into a capture that never began.
      unlistenRef.current.forEach(fn => { try { fn(); } catch { /* already gone */ } });
      unlistenRef.current = [];
      const message = `Couldn't start system audio capture: ${(e as Error).message ?? e}.${stillRecording}`;
      if (opts.micLive) setSystemAudioError(message);
      return message;
    }
  }, [stashDraft]);

  /** Stop far-side capture, keeping the listeners alive across the stop.
   *
   *  Order matters: the sidecar FLUSHES its partial buffer as a final chunk
   *  when told to stop, and that chunk arrives as an event. Unregistering the
   *  listener first — as this did — threw away the last stretch of the call,
   *  which is the part where meetings decide things.
   *
   *  Newer natives also RETURN every chunk still unread once the helper's
   *  output has been fully drained — the tail no longer depends on an event
   *  winning a race with the unsubscribe. Those paths go through the same
   *  intake as the events, after them, so each is taken once and in order
   *  (the helper names chunks `sysaudio_%04d`, and the list is sorted). An
   *  older native returns nothing and the event path alone carries the tail.
   *
   *  `takeTail: false` (discard, unmount) stops without taking anything. */
  const stopSystemAudio = useCallback(async (takeTail: boolean = true) => {
    const intake = farIntakeRef.current;
    const unlisteners = unlistenRef.current;
    const hadCapture = intake !== null || unlisteners.length > 0;
    let failure: string | null = null;
    farIntakeRef.current = null;
    // Retire only this stop's listeners. A newer start may register its own
    // while native shutdown awaits; this continuation must not remove them.
    unlistenRef.current = [];
    if ('__TAURI_INTERNALS__' in window) {
      try {
        const unread = await stopSystemAudioCapture();
        // Let the flushed chunk's event dispatch before the listeners go.
        await new Promise(resolve => setTimeout(resolve, 0));
        if (takeTail && intake && unread) unread.forEach(intake);
      } catch (e) {
        if (takeTail && hadCapture) failure = (e as Error)?.message || String(e);
      }
    }
    unlisteners.forEach(fn => { try { fn(); } catch { /* already gone */ } });
    return failure;
  }, []);

  /**
   * The helper failed mid-recording and it was the only source. Stop
   * everything — helper, listeners, timer — keep whatever it had already
   * captured (plus the notepad) as a recoverable draft, and leave 'recording'
   * with a clear error. Nothing is auto-saved: a meeting cut off by a
   * permission prompt is the user's to save or let go, not ours to file.
   */
  systemOnlyFailRef.current = async (recordingId: number, message: string) => {
    // Already ended (or ending) — the stop path owns it.
    if (recordingIdRef.current !== recordingId || stoppingRef.current) return;
    stoppingRef.current = true;
    try {
      setState('finishing');
      clearInterval(timerRef.current);
      const stopError = await stopSystemAudio(); // takes any flushed tail, then unsubscribes
      await farQueueRef.current;
      if (recordingIdRef.current !== recordingId) return;
      if (stopError) {
        farStashRef.current.parts.push(GAP_MARKER);
        setFailedChunks(n => n + 1);
      }
      stashDraft();
      const startedAt = startedAtRef.current.toISOString();
      const kept = readDrafts().find(d => d.startedAt === startedAt);
      const recoverable = kept !== undefined && draftBody(kept).length > 0;
      if (recoverable) {
        draftKeptRef.current = true;
        setRecoveredDrafts(readDrafts().filter(d => draftBody(d).length > 0));
      } else {
        clearDraft(startedAt);
      }
      targetRef.current = null;
      recordingIdRef.current += 1;
      setTarget(null);
      setActiveSources(null);
      setError(`${message} ${recoverable
        ? 'What was captured before it stopped is kept — save it from the recovered-recording card.'
        : 'Nothing had been captured yet, so there is nothing to save.'}`);
      setState('error');
    } finally {
      stoppingRef.current = false;
    }
  };

  // Unmounting mid-recording releases the mic AND the far-side helper — it is
  // a separate process and would otherwise record the call with nobody
  // listening. The draft stays, so the words are offered back next mount.
  useEffect(() => () => {
    teardownAudio();
    // Retired unconditionally: a start still awaiting the helper sees the
    // change and stops it rather than leaving it running.
    recordingIdRef.current += 1;
    if (unlistenRef.current.length > 0) void stopSystemAudio(false);
  }, [teardownAudio, stopSystemAudio]);

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
    const recordingId = recordingIdRef.current;
    const parts = partsRef.current;
    const isCurrent = () => recordingIdRef.current === recordingId && partsRef.current === parts;
    const slot = parts.length;
    parts.push(''); // reserve a slot in THIS recording, never a future recording's array

    queueRef.current = queueRef.current.then(async () => {
      if (!isCurrent()) return;
      try {
        const wav = encodeWav(merged, rate);
        const { text } = await api.transcribeAudio(new Blob([wav], { type: 'audio/wav' }));
        if (!isCurrent()) return;
        parts[slot] = text;
        if (text.trim().length > 0) setNearChunksHeard(n => n + 1);
      } catch {
        if (!isCurrent()) return;
        parts[slot] = GAP_MARKER;
        setFailedChunks(n => n + 1);
      }
      stashDraft();
    });
  }, [stashDraft]);

  const start = useCallback(async (nextTarget: MeetingTarget) => {
    if (state === 'recording' || state === 'finishing' || startingRef.current) return;
    const micWanted = recordMic;
    const app = systemAudio ? systemAudioApp : null;
    if (!micWanted && !systemAudio) {
      // Nothing to record. The picker disables Start for this; the hook
      // refuses too rather than "recording" silence into an empty note.
      setError('Choose at least one source to record.');
      setState('error');
      return;
    }
    startingRef.current = true;
    draftKeptRef.current = false;
    // A failed save left a transcript in hand. Starting over drops it from the
    // live panel, but its draft is still stashed — put it on the recovery card
    // now instead of hiding it until the next launch.
    if (unsavedRef.current) {
      setRecoveredDrafts(readDrafts().filter(d => draftBody(d).length > 0));
    }
    setError(null);
    setFailedChunks(0);
    setElapsedSeconds(0);
    partsRef.current = [];
    queueRef.current = Promise.resolve();
    setNearChunksHeard(0);
    setFarChunksHeard(0);
    setSystemAudioError(null);
    setUserNotes('');
    userNotesRef.current = '';
    // A new identity for this recording. Any far-side parts still stamped with
    // the previous one stop being visible the instant this line runs, so a
    // mic-only meeting can never inherit the last call's other side.
    const recordingId = ++recordingIdRef.current;
    unsavedRef.current = null;
    // The microphone comes FIRST. Starting the far-side helper before it meant
    // a refused mic left the helper recording the call with no recording to
    // belong to — and every chunk it delivered stashed a draft under the
    // PREVIOUS recording's start time, overwriting a stranded transcript.
    // `targetRef` is set only once the mic is live, so nothing can stash until
    // there is something to stash for.
    //
    // System audio only: no microphone is opened at all — never asked for,
    // never prompted. The helper then IS the recording, so its absence is an
    // error up front rather than a recording that silently captures nothing.
    // A chosen app is a promise of isolation. If it is not running now — or
    // this native cannot target an app — refuse rather than record something
    // else under its name, and never quietly widen to all system audio.
    if (app) {
      const running = await listSystemAudioApps();
      if (recordingIdRef.current !== recordingId) { startingRef.current = false; return; }
      if (appAvailability(app, running) !== 'available') {
        recordingIdRef.current += 1;
        setTarget(null);
        setError(unavailableAppMessage(app, running));
        setState('error');
        startingRef.current = false;
        return;
      }
    }
    const helperAvailable = micWanted || await probeSystemAudio();
    if (recordingIdRef.current !== recordingId) { startingRef.current = false; return; }
    if (!helperAvailable) {
      recordingIdRef.current += 1;
      setTarget(null);
      setError("System audio capture isn't available in this build, so there is nothing to record. Turn on “Record my microphone”, or use a build that includes the capture helper.");
      setState('error');
      startingRef.current = false;
      return;
    }
    if (micWanted) try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      // Permission may resolve after unmount/discard. Stop this returned
      // stream before assigning refs or creating any AudioContext/timer/draft.
      if (recordingIdRef.current !== recordingId) {
        stream.getTracks().forEach(track => track.stop());
        startingRef.current = false;
        return;
      }
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
    } catch (e) {
      teardownAudio();
      // Retire the identity so nothing late can file itself under it.
      recordingIdRef.current += 1;
      setTarget(null);
      setError(micStartError(e));
      setState('error');
      startingRef.current = false;
      return;
    }
    startedAtRef.current = new Date();
    // Minted once, here, and carried through every save, retry and recovery
    // of this recording — never regenerated.
    meetingIdRef.current = newRecordingId();
    setTarget(nextTarget);
    targetRef.current = nextTarget;
    // Seed the stash immediately: even a crash before the first chunk
    // leaves a record of which project the recording was for.
    stashDraft();
    timerRef.current = setInterval(() => {
      setElapsedSeconds(Math.floor((Date.now() - startedAtRef.current.getTime()) / 1000));
    }, 1000);
    setActiveSources({ mic: micWanted, system: systemAudio, app });
    // Far side last. With the mic live a failure is reported on the panel and
    // the mic keeps recording; without it, the start has failed.
    const farError = systemAudio
      ? await startSystemAudio(recordingId, { micLive: micWanted, appBundleId: app?.bundleId ?? null })
      : null;
    startingRef.current = false;
    if (recordingIdRef.current !== recordingId) {
      // Unmounted while the helper was starting: neither source may outlive us.
      teardownAudio();
      void stopSystemAudio(false);
      return;
    }
    // The helper already reported a failure and the system-only shutdown is
    // under way — it owns the state from here; do not flip back to recording.
    if (stoppingRef.current) return;
    if (farError && !micWanted) {
      // Nothing is capturing: undo the start completely — timer, seeded
      // draft, target, identity — and say why.
      clearInterval(timerRef.current);
      clearDraft(startedAtRef.current.toISOString());
      targetRef.current = null;
      recordingIdRef.current += 1;
      setTarget(null);
      setActiveSources(null);
      setError(farError);
      setState('error');
      return;
    }
    setState('recording');
  }, [
    stopSystemAudio, state, flushChunk, teardownAudio, startSystemAudio, systemAudio,
    systemAudioApp, recordMic, stashDraft, clearDraft,
  ]);

  // ── Triage of the last saved meeting ──────────────────────────────────────
  // Saving the note and the Orchestrator triaging it are separate events: the
  // save returns as soon as the note and its triage row are durable, and the
  // triage runs afterwards. This follows the receipt so the UI can say which
  // stage it is at instead of implying the work is done at save.
  const [triage, setTriage] = useState<MeetingTriageView | null>(null);
  /** Bumped to cancel whatever poll loop is running (new save, dismiss,
   *  unmount); every callback checks it before touching state. */
  const triageGenRef = useRef(0);
  const triageTimerRef = useRef<ReturnType<typeof setTimeout>>();

  const stopTriagePolling = useCallback(() => {
    triageGenRef.current += 1;
    clearTimeout(triageTimerRef.current);
  }, []);

  /** Show `meeting`'s triage and poll it, with backoff, until it settles.
   *  `fetchNow` asks the daemon immediately (a reload, a manual check);
   *  otherwise the first poll waits one delay. Bounded by MAX_TRIAGE_POLLS
   *  and MAX_TRIAGE_POLL_ERRORS — then it stops and offers a manual check. */
  const followTriage = useCallback((
    meeting: LastSavedMeeting,
    initial: MeetingTriageReceipt | null,
    fetchNow: boolean,
  ) => {
    stopTriagePolling();
    const gen = triageGenRef.current;
    const live = () => gen === triageGenRef.current;
    setTriage({ meeting, receipt: initial, unavailable: false, stalled: false, error: null });
    let polls = 0;
    let errors = 0;
    const tick = async () => {
      if (!live()) return;
      polls += 1;
      try {
        const receipt = await api.getMeetingTriage(meeting.projectId, meeting.recordingId);
        if (!live()) return;
        errors = 0;
        setTriage(v => v && { ...v, receipt, unavailable: false, stalled: false, error: null });
        if (isTriageSettled(receipt.status)) return;
      } catch (e) {
        if (!live()) return;
        if ((e as { status?: number }).status === 404) {
          setTriage(v => v && { ...v, unavailable: true });
          return;
        }
        errors += 1;
        if (errors >= MAX_TRIAGE_POLL_ERRORS) {
          setTriage(v => v && { ...v, stalled: true, error: (e as Error).message || 'request failed' });
          return;
        }
      }
      if (polls >= MAX_TRIAGE_POLLS) {
        setTriage(v => v && { ...v, stalled: true });
        return;
      }
      triageTimerRef.current = setTimeout(() => void tick(), triagePollDelay(polls));
    };
    if (initial && isTriageSettled(initial.status) && !fetchNow) return;
    if (fetchNow) void tick();
    else triageTimerRef.current = setTimeout(() => void tick(), triagePollDelay(0));
  }, [stopTriagePolling]);

  /** Record that a meeting note landed, and start following its triage. A
   *  daemon that returned no receipt predates triage: say so, don't poll. */
  const noteSaved = useCallback((meeting: LastSavedMeeting, receipt: MeetingTriageReceipt | undefined) => {
    if (!receipt) {
      stopTriagePolling();
      setTriage({ meeting, receipt: null, unavailable: true, stalled: false, error: null });
      return;
    }
    writeLastSaved(meeting);
    followTriage(meeting, receipt, false);
  }, [followTriage, stopTriagePolling]);

  // After a reload, pick the last saved meeting's triage back up. Unmount
  // cancels any poll in flight.
  useEffect(() => {
    const last = readLastSaved();
    if (last) followTriage(last, null, true);
    return () => stopTriagePolling();
  }, [followTriage, stopTriagePolling]);

  /** Ask the daemon to re-run a failed or skipped triage. */
  const retryTriage = useCallback(async (): Promise<boolean> => {
    const meeting = triage?.meeting;
    if (!meeting) return false;
    try {
      const receipt = await api.retryMeetingTriage(meeting.projectId, meeting.recordingId);
      followTriage(meeting, receipt, false);
      return true;
    } catch (e) {
      setTriage(v => v && { ...v, error: `Couldn't retry the triage: ${(e as Error).message || 'request failed'}` });
      return false;
    }
  }, [triage, followTriage]);

  /** Check the triage again now (after polling gave up). */
  const refreshTriage = useCallback(() => {
    if (triage) followTriage(triage.meeting, triage.receipt, true);
  }, [triage, followTriage]);

  /** Stop following the last meeting's triage. The triage itself carries on. */
  const dismissTriage = useCallback(() => {
    stopTriagePolling();
    setTriage(null);
    clearLastSaved();
  }, [stopTriagePolling]);

  const saveNote = useCallback(
    async (projectId: string, title: string, body: string, recordingId: string) => {
      // kind:'meeting' hands the note to the daemon's durable Orchestrator
      // triage. `recordingId` makes the save idempotent: a retry after a lost
      // response comes back as the same note, not a second one.
      const created = await api.createProjectNote(projectId, { title, body, kind: 'meeting', recordingId });
      unsavedRef.current = null;
      clearDraft(startedAtRef.current.toISOString());
      // Saved and cleared — a late chunk must not resurrect the draft.
      targetRef.current = null;
      return created;
    },
    [clearDraft]
  );

  /** One save attempt. Failure keeps the transcript for `retrySave`; a save
   *  already in flight makes a second attempt a no-op, not a second note. */
  const attemptSave = useCallback(
    async (t: MeetingTarget, title: string, body: string, recordingId: string): Promise<SavedNoteRef | null> => {
      if (savingRef.current) return null;
      savingRef.current = true;
      const { projectId } = t;
      try {
        const created = await saveNote(projectId, title, body, recordingId);
        const noteId = created.id;
        // Usage-only signal (no transcript in the payload) — mirrors useDictation.
        emitActivity('dictation_completed', 'voice', { char_count: body.length });
        noteSaved({
          projectId, projectName: t.projectName, recordingId, noteId,
          savedAt: new Date().toISOString(),
        }, created.triage);
        setState('idle');
        setTarget(null);
        return { projectId, noteId };
      } catch (e) {
        setError(`Couldn't save the meeting note: ${(e as Error).message || 'request failed'}. Your transcript is kept — retry the save.`);
        setState('error');
        return null;
      } finally {
        savingRef.current = false;
      }
    },
    [saveNote, noteSaved]
  );

  /** Stop recording, transcribe the tail, and save the note. Resolves to the
   *  saved note's identity so callers can deep-link to it, or null. */
  const stop = useCallback(async (): Promise<SavedNoteRef | null> => {
    if (state !== 'recording' || !target || stoppingRef.current) return null;
    stoppingRef.current = true;
    try {
      setState('finishing');
      flushChunk(); // tail
      teardownAudio();

      const farStopError = await stopSystemAudio();
      await queueRef.current;      // mic chunks transcribed (or gap-marked)
      await farQueueRef.current;   // far-side chunks likewise
      if (farStopError) {
        farStashRef.current.parts.push(GAP_MARKER);
        setFailedChunks(n => n + 1);
        setSystemAudioError(farStopError);
      }
      const transcript = composeTranscriptBody(partsRef.current, liveFarParts());
      const composed = meetingNoteBody(transcript, userNotesRef.current);
      if (!composed) {
        // Nothing heard and nothing typed — honest outcome, no empty note.
        clearDraft(startedAtRef.current.toISOString());
        targetRef.current = null;
        setError('No speech was transcribed — no note was saved.');
        setState('error');
        return null;
      }
      const title = meetingNoteTitle(startedAtRef.current);
      const recordingId = meetingIdRef.current;
      // Freeze the exact title and body (and the latest notes) into the draft
      // BEFORE the save can fail or its response be lost: a recovery then
      // re-sends the same bytes under the same id, which the daemon treats as
      // the same note.
      stashDraft({ title, body: composed });
      unsavedRef.current = { title, body: composed, recordingId };
      if (farStopError) {
        setError(`Recording stopped, but final system audio may be incomplete (${farStopError}). Your captured transcript and notes are kept. Retry save will save this partial transcript with a gap marker.`);
        setState('error');
        return null;
      }
      return await attemptSave(target, title, composed, recordingId);
    } finally {
      stoppingRef.current = false;
    }
  }, [state, target, flushChunk, teardownAudio, stopSystemAudio, liveFarParts, attemptSave, stashDraft, clearDraft]);

  /** Re-attempt the note save after a failure (transcript was retained). */
  const retrySave = useCallback(async (): Promise<SavedNoteRef | null> => {
    const unsaved = unsavedRef.current;
    if (!unsaved || !target || savingRef.current) return null;
    setError(null);
    // The SAME recording id as the failed attempt — never a fresh one: if the
    // first save landed and only its response was lost, this returns it.
    return attemptSave(target, unsaved.title, unsaved.body, unsaved.recordingId);
  }, [target, attemptSave]);

  /** Discard the recording (and any unsaved transcript) without saving. */
  const discard = useCallback(() => {
    teardownAudio();
    // The far-side helper is a SEPARATE process and does not stop with the
    // mic: without this it kept recording the call after the user discarded,
    // and each chunk it delivered re-created the draft they had just thrown
    // away. `targetRef` is nulled first so any continuation already queued on
    // `farQueueRef` finds nothing to stash.
    targetRef.current = null;
    void stopSystemAudio(false);
    bufferRef.current = [];
    bufferedRef.current = 0;
    partsRef.current = [];
    unsavedRef.current = null;
    // Retire this recording's identity: any far-side chunk still in flight is
    // now stamped to a recording that no longer exists and is ignored.
    recordingIdRef.current += 1;
    setFarChunksHeard(0);
    setSystemAudioError(null);
    setNearChunksHeard(0);
    setUserNotes('');
    // A draft already handed to the recovery card is the user's to save or
    // dismiss there — closing the error panel must not delete it.
    if (!draftKeptRef.current) clearDraft(startedAtRef.current.toISOString());
    setTarget(null);
    setError(null);
    setState('idle');
  }, [teardownAudio, clearDraft, stopSystemAudio]);

  /** Save a crash-recovered transcript as the meeting note it should have
   *  been. Same path as a live save, so the kanban extraction runs too. */
  const recoverDraft = useCallback(async (d: MeetingDraft): Promise<SavedNoteRef | null> => {
    const body = draftBody(d);
    const title = d.title ?? `${meetingNoteTitle(new Date(d.startedAt))} (recovered)`;
    // The id the recording was minted with (or, for an old draft, one derived
    // from its start time) — so a meeting whose live save actually landed is
    // matched to that note instead of filed twice.
    const recordingId = draftRecordingId(d);
    try {
      const created = await api.createProjectNote(d.projectId, { title, body, kind: 'meeting', recordingId });
      emitActivity('dictation_completed', 'voice', { char_count: body.length });
      clearDraft(d.startedAt);
      setRecoveredDrafts(list => list.filter(x => x.startedAt !== d.startedAt));
      noteSaved({
        projectId: d.projectId, projectName: d.projectName, recordingId, noteId: created.id,
        savedAt: new Date().toISOString(),
      }, created.triage);
      return { projectId: d.projectId, noteId: created.id };
    } catch (e) {
      setError(`Couldn't save the recovered transcript: ${(e as Error).message || 'request failed'}.`);
      return null;
    }
  }, [clearDraft, noteSaved]);

  /** Let one recovered transcript go. */
  const dismissDraft = useCallback((d: MeetingDraft) => {
    clearDraft(d.startedAt);
    setRecoveredDrafts(list => list.filter(x => x.startedAt !== d.startedAt));
  }, [clearDraft]);

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
    /** Record this machine's microphone (default on). */
    recordMic,
    setRecordMic,
    /** One app to capture instead of all system audio (null = all). */
    systemAudioApp,
    setSystemAudioApp,
    /** Running apps this native can capture alone; null = it can't. */
    listSystemAudioApps,
    /** What the current recording captures, fixed at Start. */
    activeSources,
    /** Far-side chunks transcribed to words during this recording. */
    farChunksHeard,
    /** Mic chunks that transcribed to words — proof the microphone is live. */
    nearChunksHeard,
    /** The notepad the user types into while recording. */
    userNotes,
    setUserNotes,
    /** Transcripts stranded by a crash/quit, newest first, awaiting save or
     *  dismissal. One slot per recording — a later meeting never buries an
     *  earlier one that was not recovered. */
    recoveredDrafts,
    recoverDraft,
    dismissDraft,
    /** The last saved meeting and where its Orchestrator triage stands —
     *  separate from the save, which is done by the time this appears. */
    triage,
    retryTriage,
    refreshTriage,
    dismissTriage,
  };
}
