/**
 * @vitest-environment jsdom
 *
 * useMeetingDictation — the real recording lifecycle, mocked only at the audio
 * and IPC boundary: getUserMedia/AudioContext (the mic), the Tauri
 * `invoke`/`listen` pair (the ScreenCaptureKit helper), and the two daemon
 * calls (local transcription, project-note save). Everything between — chunk
 * cutting, ordering, provenance, drafts, guards — is the code under test.
 *
 * The regressions pinned here all lose or misfile a meeting quietly: a helper
 * left recording after the mic was refused, a tail chunk missing from the
 * note, a second note from a double click, one call's far side in the next
 * call's note, and notes thrown away because the machine heard nothing.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';

const { transcribeAudio, createProjectNote, getMeetingTriage, retryMeetingTriage, invoke, listen } = vi.hoisted(() => ({
  transcribeAudio: vi.fn(),
  createProjectNote: vi.fn(),
  getMeetingTriage: vi.fn(),
  retryMeetingTriage: vi.fn(),
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock('../lib/api', () => ({
  api: { transcribeAudio, createProjectNote, getMeetingTriage, retryMeetingTriage },
}));
const { readiness } = vi.hoisted(() => ({ readiness: {
  status: 'ready', message: 'Local model found; inference is not yet verified', retry: vi.fn(async () => {}),
} }));
vi.mock('../lib/dictationReadiness', () => ({
  useDictationReadiness: () => readiness,
  getDictationReadiness: () => readiness,
}));
vi.mock('../lib/emitActivity', () => ({ emitActivity: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

import {
  GAP_MARKER,
  MAX_TRIAGE_POLL_ERRORS,
  MEETING_CHUNK_SECONDS,
  NOTES_STASH_DEBOUNCE_MS,
  NO_SPEECH_MARKER,
  TRIAGE_POLL_DELAYS_MS,
  draftKey,
  readDrafts,
  useMeetingDictation,
} from './useMeetingDictation';
import type { MeetingTriageReceipt } from '../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const RATE = 16000;
const PROJECT_A = { projectId: 'proj-a', projectName: 'Synthetic A' };
const PROJECT_B = { projectId: 'proj-b', projectName: 'Synthetic B' };

// ── Mic boundary ────────────────────────────────────────────────────────────

class FakeTrack { stopped = false; stop() { this.stopped = true; } }

let tracks: FakeTrack[] = [];
let proc: { onaudioprocess: ((e: unknown) => void) | null; connect: () => void; disconnect: () => void } | null = null;
let contexts: FakeAudioContext[] = [];
let failAudioContext = false;
let contextInitiallySuspended = false;
let resumeAudioContext = vi.fn(async () => {});

class FakeAudioContext {
  sampleRate = RATE;
  state = contextInitiallySuspended ? 'suspended' : 'running';
  destination = {};
  constructor() {
    if (failAudioContext) throw new Error('audio device busy');
    contexts.push(this);
  }
  createMediaStreamSource() { return { connect: () => {} }; }
  createScriptProcessor() {
    proc = { onaudioprocess: null, connect: () => {}, disconnect: () => {} };
    return proc;
  }
  async resume() { await resumeAudioContext(); this.state = 'running'; }
  close() { this.state = 'closed'; return Promise.resolve(); }
}

/** Feed `seconds` of mic audio through the live ScriptProcessor. */
function speak(seconds: number) {
  const total = Math.round(seconds * RATE);
  for (let fed = 0; fed < total; fed += 4096) {
    const n = Math.min(4096, total - fed);
    proc!.onaudioprocess!({ inputBuffer: { getChannelData: () => new Float32Array(n) } });
  }
}

// ── Helper (Tauri) boundary ─────────────────────────────────────────────────

let handlers: Record<string, Array<(e: { payload: unknown }) => void>> = {};
let invokeLog: string[] = [];
/** Paths the helper flushes as its final chunk when told to stop. */
let flushOnStop: string[] = [];
let failStartSystemAudio = false;
/** What `stop_system_audio` RETURNS: the newer native lists unread chunk
 *  paths; `undefined` models an older native that returns nothing. Consumed
 *  by the next stop. */
let stopReturns: Array<string[] | undefined | {unreadPaths: string[]; error: string | null}> = [];
/** `system_audio_available`. */
let helperAvailable = true;
/** `list_system_audio_apps`; 'missing' models a native without the command. */
let runningApps: Array<{ bundleId: string; name: string }> | 'missing' = [];

function emit(event: string, payload: unknown) {
  (handlers[event] ?? []).slice().forEach(h => h({ payload }));
}

/** Transcription keyed by what was sent: far-side chunks carry their path in
 *  the bytes we return from `read_audio_chunk`; mic chunks are queued. */
let micTexts: Array<string | Error> = [];
const farTexts = new Map<string, string>();

/** jsdom's Blob has no `.text()`. */
function blobText(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsText(blob);
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  readiness.status = 'ready'; readiness.message = 'Local model found';
  localStorage.clear();
  tracks = []; proc = null; contexts = []; failAudioContext = false;
  contextInitiallySuspended = false; resumeAudioContext = vi.fn(async () => {});
  handlers = {}; invokeLog = []; flushOnStop = []; failStartSystemAudio = false; stopReturns = []; helperAvailable = true;
  runningApps = [{ bundleId: 'ai.permagent.record-practice-20261007', name: 'Record Practice' }];
  micTexts = []; farTexts.clear();

  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  (globalThis as Record<string, unknown>).AudioContext = FakeAudioContext;
  Object.defineProperty(navigator, 'mediaDevices', {
    configurable: true,
    value: {
      getUserMedia: vi.fn(async () => {
        const t = new FakeTrack();
        tracks.push(t);
        return { getTracks: () => [t] };
      }),
    },
  });

  listen.mockImplementation(async (event: string, h: (e: { payload: unknown }) => void) => {
    (handlers[event] ??= []).push(h);
    return () => { handlers[event] = (handlers[event] ?? []).filter(x => x !== h); };
  });
  invoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    invokeLog.push(cmd);
    if (cmd === 'system_audio_available') return helperAvailable;
    if (cmd === 'list_system_audio_apps') {
      if (runningApps === 'missing') throw new Error('command list_system_audio_apps not found');
      return runningApps;
    }
    if (cmd === 'start_system_audio' && failStartSystemAudio) throw new Error('helper missing');
    if (cmd === 'stop_system_audio') {
      flushOnStop.splice(0).forEach(p => emit('system_audio_chunk', p));
      return stopReturns.shift();
    }
    if (cmd === 'read_audio_chunk') {
      return Array.from(new TextEncoder().encode(`far:${args!.path as string}`));
    }
    return undefined;
  });
  transcribeAudio.mockImplementation(async (blob: Blob) => {
    const text = await blobText(blob);
    if (text.startsWith('far:')) return { text: farTexts.get(text.slice(4)) ?? '' };
    const next = micTexts.shift();
    if (next instanceof Error) throw next;
    return { text: next ?? '' };
  });
  createProjectNote.mockImplementation(async () => ({ id: `note-${createProjectNote.mock.calls.length}` }));
});

afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

// ── Hook harness ────────────────────────────────────────────────────────────

type Hook = ReturnType<typeof useMeetingDictation>;

function mount() {
  const ref: { current: Hook | null } = { current: null };
  function Probe() { ref.current = useMeetingDictation(); return null; }
  const container = document.createElement('div');
  let root: Root;
  act(() => { root = createRoot(container); root.render(<Probe />); });
  return {
    get h() { return ref.current!; },
    unmount: () => act(() => root.unmount()),
  };
}

/** Run an async hook action inside act and settle every queued microtask. */
async function run<T>(fn: () => Promise<T>): Promise<T> {
  let out!: T;
  await act(async () => { out = await fn(); });
  return out;
}

async function settle() {
  await act(async () => { await new Promise(r => setTimeout(r, 0)); });
}

const noteBody = (call = 0) => createProjectNote.mock.calls[call][1].body as string;
const sentRecordingId = (call = 0) => createProjectNote.mock.calls[call][1].recordingId as string;
/** Far-side chunk paths that were actually read, in order. */
const farReads = () => invoke.mock.calls.filter(c => c[0] === 'read_audio_chunk').map(c => c[1].path as string);

// ── Tests ───────────────────────────────────────────────────────────────────

describe('start → stop, mic only', () => {
  it('saves the tail chunk as a kind=meeting note on the chosen project', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    expect(r.h.state).toBe('recording');

    micTexts = ['we agreed to ship friday'];
    speak(3); // well under a chunk: only the stop-time flush carries it
    const saved = await run(() => r.h.stop());

    expect(createProjectNote).toHaveBeenCalledTimes(1);
    const [projectId, note] = createProjectNote.mock.calls[0];
    expect(projectId).toBe(PROJECT_A.projectId);
    expect(note.kind).toBe('meeting');
    expect(note.body).toBe('we agreed to ship friday');
    // Mic-only stays single-voice — no "You:" implying the far side was heard.
    expect(note.body).not.toContain('**You:**');
    expect(saved).toEqual({ projectId: PROJECT_A.projectId, noteId: 'note-1' });
    expect(r.h.state).toBe('idle');
    expect(tracks.every(t => t.stopped)).toBe(true);
    expect(contexts.every(c => c.state === 'closed')).toBe(true);
    expect(readDrafts()).toEqual([]);
  });

  it('keeps full chunks and the tail in speech order, gap-marking a failed one', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    micTexts = ['first', new Error('whisper down'), 'tail'];
    speak(MEETING_CHUNK_SECONDS);
    speak(MEETING_CHUNK_SECONDS);
    speak(2);
    await run(() => r.h.stop());
    expect(noteBody()).toBe(`first ${GAP_MARKER} tail`);
    expect(r.h.failedChunks).toBe(1);
  });

  it('waits for a slow tail transcription before saving', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    let release!: (v: { text: string }) => void;
    transcribeAudio.mockImplementationOnce(() => new Promise(res => { release = res; }));
    speak(1);
    let done = false;
    let stopping!: Promise<unknown>;
    act(() => { stopping = r.h.stop().then(() => { done = true; }); });
    await settle();
    expect(r.h.state).toBe('finishing');
    expect(createProjectNote).not.toHaveBeenCalled();
    await act(async () => { release({ text: 'last words' }); await stopping; });
    expect(done).toBe(true);
    expect(noteBody()).toBe('last words');
  });
});

describe('notes', () => {
  it('puts the notepad verbatim ahead of the transcript under the daemon headings', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    act(() => r.h.setUserNotes('- pricing??\n- renewal'));
    micTexts = ['we covered pricing'];
    speak(1);
    await run(() => r.h.stop());
    expect(noteBody()).toBe('## Your notes\n\n- pricing??\n- renewal\n\n## Transcript\n\nwe covered pricing');
  });

  it('saves notes even when nothing was transcribed, saying so plainly', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    act(() => r.h.setUserNotes('decide vendor by Monday'));
    speak(1); // transcribes to ''
    await run(() => r.h.stop());
    expect(noteBody()).toBe(`## Your notes\n\ndecide vendor by Monday\n\n## Transcript\n\n${NO_SPEECH_MARKER}`);
  });

  it('saves nothing and says why when there was neither speech nor notes', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    speak(1);
    const saved = await run(() => r.h.stop());
    expect(saved).toBeNull();
    expect(createProjectNote).not.toHaveBeenCalled();
    expect(r.h.state).toBe('error');
    expect(r.h.error).toMatch(/No speech was transcribed/);
    expect(readDrafts()).toEqual([]);
  });

  it('stashes the notepad between chunks, so a crash does not lose typing', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    act(() => r.h.setUserNotes('typed between chunks'));
    await act(async () => { await new Promise(res => setTimeout(res, NOTES_STASH_DEBOUNCE_MS + 50)); });
    expect(readDrafts()[0]?.userNotes).toBe('typed between chunks');
    r.unmount();
  });
});

describe('both sides', () => {
  it('interleaves the far side as "Others", including the chunk flushed at stop', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    expect(invokeLog).toContain('start_system_audio');

    farTexts.set('/tmp/sysaudio_0.wav', 'welcome everyone');
    farTexts.set('/tmp/sysaudio_1.wav', 'any blockers');
    act(() => emit('system_audio_chunk', '/tmp/sysaudio_0.wav'));
    flushOnStop = ['/tmp/sysaudio_1.wav'];
    micTexts = ['thanks for having me', 'none from me'];
    speak(MEETING_CHUNK_SECONDS);
    speak(1);
    await run(() => r.h.stop());

    expect(noteBody()).toBe(
      '**Others:** welcome everyone\n\n**You:** thanks for having me\n\n' +
      '**Others:** any blockers\n\n**You:** none from me',
    );
    // Per-side labels only — mixed system audio is never attributed to a name.
    expect(noteBody().match(/\*\*[^*]+:\*\*/g)!.every(l => l === '**Others:**' || l === '**You:**')).toBe(true);
    expect(handlers.system_audio_chunk ?? []).toHaveLength(0);
  });

  it('never starts the helper when the mic is refused', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    (navigator.mediaDevices.getUserMedia as ReturnType<typeof vi.fn>)
      .mockRejectedValueOnce(Object.assign(new Error('denied'), { name: 'NotAllowedError' }));
    await run(() => r.h.start(PROJECT_A));

    expect(r.h.state).toBe('error');
    expect(r.h.error).toBe('Microphone access was blocked');
    expect(r.h.target).toBeNull();
    expect(invokeLog).not.toContain('start_system_audio');
    expect(handlers.system_audio_chunk ?? []).toHaveLength(0);
    expect(readDrafts()).toEqual([]);
  });

  it('releases the mic when the audio graph fails after the mic was granted', async () => {
    const r = mount();
    failAudioContext = true;
    await run(() => r.h.start(PROJECT_A));
    expect(r.h.state).toBe('error');
    expect(r.h.error).toMatch(/Couldn't start the microphone/);
    expect(tracks).toHaveLength(1);
    expect(tracks[0].stopped).toBe(true);
  });

  it('keeps recording the mic and drops its listeners when the helper fails to start', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    failStartSystemAudio = true;
    await run(() => r.h.start(PROJECT_A));
    expect(r.h.state).toBe('recording');
    expect(r.h.systemAudioError).toMatch(/Your microphone is still being recorded/);
    expect(handlers.system_audio_chunk ?? []).toHaveLength(0);
    micTexts = ['solo'];
    speak(1);
    await run(() => r.h.stop());
    expect(noteBody()).toBe('solo');
  });

  it('stops a helper left running by a previous webview before starting a new one', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    const first = invokeLog.indexOf('stop_system_audio');
    expect(first).toBeGreaterThanOrEqual(0);
    expect(first).toBeLessThan(invokeLog.indexOf('start_system_audio'));
    r.unmount();
  });

  it('stops the helper when unmounted mid-recording', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    invokeLog = [];
    r.unmount();
    await settle();
    expect(invokeLog).toContain('stop_system_audio');
    expect(tracks[0].stopped).toBe(true);
  });
});

describe('save failure, retry and races', () => {
  it('keeps the transcript and notes on a failed save, and retry saves the same body once', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    act(() => r.h.setUserNotes('follow up with legal'));
    micTexts = ['legal needs the contract'];
    speak(1);
    createProjectNote.mockRejectedValueOnce(new Error('daemon offline'));
    const first = await run(() => r.h.stop());

    expect(first).toBeNull();
    expect(r.h.state).toBe('error');
    expect(r.h.hasUnsavedTranscript).toBe(true);
    expect(r.h.error).toMatch(/daemon offline.*transcript is kept/);
    // Durable across a quit: the draft carries words AND the latest notes.
    const [draft] = readDrafts();
    expect(draft.parts).toEqual(['legal needs the contract']);
    expect(draft.userNotes).toBe('follow up with legal');

    const [a, b] = await run(() => Promise.all([r.h.retrySave(), r.h.retrySave()]));
    expect([a, b].filter(Boolean)).toHaveLength(1);
    expect(createProjectNote).toHaveBeenCalledTimes(2); // the failure + ONE retry
    expect(createProjectNote.mock.calls[1]).toEqual(createProjectNote.mock.calls[0]);
    expect(createProjectNote.mock.calls[1][0]).toBe(PROJECT_A.projectId);
    expect(r.h.state).toBe('idle');
    expect(readDrafts()).toEqual([]);
  });

  it('a double Stop saves one note', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    micTexts = ['once'];
    speak(1);
    const results = await run(() => Promise.all([r.h.stop(), r.h.stop()]));
    expect(createProjectNote).toHaveBeenCalledTimes(1);
    expect(results.filter(Boolean)).toHaveLength(1);
  });

  it('a double Start opens one microphone', async () => {
    const r = mount();
    await run(() => Promise.all([r.h.start(PROJECT_A), r.h.start(PROJECT_B)]));
    expect(navigator.mediaDevices.getUserMedia).toHaveBeenCalledTimes(1);
    expect(r.h.target).toEqual(PROJECT_A);
    r.unmount();
  });

  it('starting over after a failed save puts the stranded transcript on the recovery card', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    micTexts = ['stranded words'];
    speak(1);
    createProjectNote.mockRejectedValueOnce(new Error('offline'));
    await run(() => r.h.stop());
    expect(r.h.recoveredDrafts).toEqual([]);

    await run(() => r.h.start(PROJECT_B));
    expect(r.h.recoveredDrafts.map(d => d.projectId)).toEqual([PROJECT_A.projectId]);
    expect(r.h.recoveredDrafts[0].parts).toEqual(['stranded words']);
    r.unmount();
  });

  it('recovering a draft saves it as a kind=meeting note on ITS project', async () => {
    localStorage.setItem(draftKey('2026-10-07T10:00:00.000Z'), JSON.stringify({
      projectId: PROJECT_B.projectId, projectName: PROJECT_B.projectName,
      startedAt: '2026-10-07T10:00:00.000Z', parts: [], farParts: [], userNotes: 'only notes survived',
    }));
    const r = mount();
    expect(r.h.recoveredDrafts).toHaveLength(1); // a notes-only draft is still offered
    const saved = await run(() => r.h.recoverDraft(r.h.recoveredDrafts[0]));
    expect(saved?.projectId).toBe(PROJECT_B.projectId);
    expect(createProjectNote.mock.calls[0][0]).toBe(PROJECT_B.projectId);
    expect(createProjectNote.mock.calls[0][1].kind).toBe('meeting');
    expect(noteBody()).toContain('only notes survived');
    expect(r.h.recoveredDrafts).toEqual([]);
  });
});

describe('per-recording isolation', () => {
  it('a mic-only recording never inherits the previous call\'s far side or project', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    farTexts.set('/tmp/sysaudio_0.wav', 'confidential roadmap');
    act(() => emit('system_audio_chunk', '/tmp/sysaudio_0.wav'));
    // Keep a handle on A's listener: an event already dispatched to it after
    // A ends must be ignored, not filed into B.
    const staleA = handlers.system_audio_chunk[0];
    micTexts = ['meeting a'];
    speak(1);
    await run(() => r.h.stop());
    expect(createProjectNote.mock.calls[0][0]).toBe(PROJECT_A.projectId);
    expect(noteBody(0)).toContain('confidential roadmap');

    act(() => r.h.setSystemAudio(false));
    await run(() => r.h.start(PROJECT_B));
    farTexts.set('/tmp/sysaudio_9.wav', 'late chunk from a');
    act(() => staleA({ payload: '/tmp/sysaudio_9.wav' }));
    await settle();
    micTexts = ['meeting b'];
    speak(1);
    await run(() => r.h.stop());

    expect(createProjectNote.mock.calls[1][0]).toBe(PROJECT_B.projectId);
    expect(noteBody(1)).toBe('meeting b');
    expect(r.h.farChunksHeard).toBe(0);
  });

  it('discard stops the helper and leaves no draft behind', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    farTexts.set('/tmp/sysaudio_0.wav', 'should vanish');
    flushOnStop = ['/tmp/sysaudio_0.wav'];
    invokeLog = [];
    act(() => r.h.discard());
    await settle();
    expect(invokeLog).toContain('stop_system_audio');
    expect(readDrafts()).toEqual([]);
    expect(r.h.state).toBe('idle');
    expect(createProjectNote).not.toHaveBeenCalled();
  });
});

describe('stable recordingId', () => {
  it('is minted at Start, stashed in the draft, and sent with the live save', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    const minted = readDrafts()[0].recordingId;
    expect(minted).toBeTruthy();
    micTexts = ['hello'];
    speak(1);
    await run(() => r.h.stop());
    expect(sentRecordingId()).toBe(minted);
  });

  it('a new recording gets a new id', async () => {
    const r = mount();
    for (const p of [PROJECT_A, PROJECT_B]) {
      await run(() => r.h.start(p));
      micTexts = ['words'];
      speak(1);
      await run(() => r.h.stop());
    }
    expect(sentRecordingId(0)).not.toBe(sentRecordingId(1));
  });

  it('survives a lost save response: retry re-sends the same id and the same bytes', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    micTexts = ['decisions'];
    speak(1);
    // The daemon may have saved it; the client cannot know — only that the
    // response never arrived.
    createProjectNote.mockRejectedValueOnce(new TypeError('Failed to fetch'));
    await run(() => r.h.stop());
    await run(() => r.h.retrySave());
    expect(createProjectNote).toHaveBeenCalledTimes(2);
    expect(sentRecordingId(1)).toBe(sentRecordingId(0));
    expect(noteBody(1)).toBe(noteBody(0));
  });

  it('survives a quit after a failed save: recovery sends the same id and frozen body', async () => {
    const first = mount();
    await run(() => first.h.start(PROJECT_A));
    act(() => first.h.setUserNotes('last-second note'));
    micTexts = ['the transcript'];
    speak(1);
    createProjectNote.mockRejectedValueOnce(new TypeError('Failed to fetch'));
    await run(() => first.h.stop());
    first.unmount(); // the app quits with the save unconfirmed

    const second = mount();
    expect(second.h.recoveredDrafts).toHaveLength(1);
    await run(() => second.h.recoverDraft(second.h.recoveredDrafts[0]));
    expect(sentRecordingId(1)).toBe(sentRecordingId(0));
    expect(noteBody(1)).toBe(noteBody(0));
    expect(createProjectNote.mock.calls[1][1].title).toBe(createProjectNote.mock.calls[0][1].title);
  });

  it('a legacy draft without an id recovers under one derived from its start, every attempt', async () => {
    const startedAt = '2026-10-07T09:00:00.000Z';
    localStorage.setItem(draftKey(startedAt), JSON.stringify({
      projectId: PROJECT_A.projectId, projectName: PROJECT_A.projectName,
      startedAt, parts: ['old words'], farParts: [],
    }));
    const r = mount();
    createProjectNote.mockRejectedValueOnce(new Error('offline'));
    await run(() => r.h.recoverDraft(r.h.recoveredDrafts[0]));
    await run(() => r.h.recoverDraft(r.h.recoveredDrafts[0]));
    expect(sentRecordingId(0)).toBe(`legacy-${startedAt}`);
    expect(sentRecordingId(1)).toBe(`legacy-${startedAt}`);
  });
});

describe('far-side tail from the stop return', () => {
  it('takes every returned path once, deduped against events, in chunk order', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    farTexts.set('/tmp/sysaudio_0000.wav', 'zero');
    farTexts.set('/tmp/sysaudio_0001.wav', 'one');
    farTexts.set('/tmp/sysaudio_0002.wav', 'two');
    act(() => emit('system_audio_chunk', '/tmp/sysaudio_0000.wav'));
    // The flush arrives as an event too, AND the newer native lists it (plus
    // one whose event never fired) in its return value.
    flushOnStop = ['/tmp/sysaudio_0001.wav'];
    stopReturns = [['/tmp/sysaudio_0000.wav', '/tmp/sysaudio_0001.wav', '/tmp/sysaudio_0002.wav']];
    micTexts = ['a', 'b', 'c'];
    speak(MEETING_CHUNK_SECONDS);
    speak(MEETING_CHUNK_SECONDS);
    speak(1);
    await run(() => r.h.stop());

    expect(farReads()).toEqual(['/tmp/sysaudio_0000.wav', '/tmp/sysaudio_0001.wav', '/tmp/sysaudio_0002.wav']);
    expect(r.h.farChunksHeard).toBe(3);
    expect(noteBody()).toBe(
      '**Others:** zero\n\n**You:** a\n\n**Others:** one\n\n**You:** b\n\n**Others:** two\n\n**You:** c',
    );
  });

  it('works when an older native returns nothing — the event path carries the tail', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    farTexts.set('/tmp/sysaudio_0000.wav', 'tail via event');
    flushOnStop = ['/tmp/sysaudio_0000.wav'];
    stopReturns = [undefined];
    micTexts = ['me'];
    speak(1);
    await run(() => r.h.stop());
    expect(farReads()).toEqual(['/tmp/sysaudio_0000.wav']);
    expect(noteBody()).toContain('**Others:** tail via event');
  });

  it('does not take paths an orphaned capture returns at Start', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    stopReturns = [['/tmp/old/sysaudio_0007.wav']]; // the pre-start reset
    await run(() => r.h.start(PROJECT_A));
    micTexts = ['fresh'];
    speak(1);
    await run(() => r.h.stop());
    expect(farReads()).toEqual([]);
    expect(noteBody()).toBe('fresh');
  });

  it('discard stops without taking the returned tail', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    stopReturns = [['/tmp/sysaudio_0000.wav']];
    act(() => r.h.discard());
    await settle();
    expect(farReads()).toEqual([]);
  });

  it('a stale intake from a finished recording cannot feed the next one', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    const staleA = handlers.system_audio_chunk[0];
    micTexts = ['a'];
    speak(1);
    await run(() => r.h.stop());

    await run(() => r.h.start(PROJECT_B)); // system audio still on: a new intake
    farTexts.set('/tmp/sysaudio_0000.wav', 'belongs to A');
    act(() => staleA({ payload: '/tmp/sysaudio_0000.wav' }));
    farTexts.set('/tmp/b/sysaudio_0000.wav', 'belongs to B');
    act(() => emit('system_audio_chunk', '/tmp/b/sysaudio_0000.wav'));
    micTexts = ['b'];
    speak(1);
    await run(() => r.h.stop());
    expect(noteBody(1)).toContain('belongs to B');
    expect(noteBody(1)).not.toContain('belongs to A');
  });
});

// ── Triage ──────────────────────────────────────────────────────────────────

function receipt(over: Partial<MeetingTriageReceipt> = {}): MeetingTriageReceipt {
  return {
    id: 'tri-1', projectId: PROJECT_B.projectId, recordingId: 'rec-1', noteId: 'note-1',
    status: 'pending', attemptCount: 0, cardIds: [], truncated: false, created: true,
    ...over,
  };
}

/** Seed a crash draft and recover it — a save with no audio in the way, so
 *  the triage tests can run on fake timers. */
async function saveViaRecovery(r: ReturnType<typeof mount>, triage: MeetingTriageReceipt | undefined) {
  createProjectNote.mockResolvedValueOnce({ id: 'note-1', triage });
  await run(() => r.h.recoverDraft(r.h.recoveredDrafts[0]));
}

function seedDraft() {
  localStorage.setItem(draftKey('2026-10-07T10:00:00.000Z'), JSON.stringify({
    projectId: PROJECT_B.projectId, projectName: PROJECT_B.projectName,
    startedAt: '2026-10-07T10:00:00.000Z', parts: ['ship it'], farParts: [], recordingId: 'rec-1',
  }));
}

async function advance(ms: number) {
  await act(async () => { await vi.advanceTimersByTimeAsync(ms); });
}

describe('triage status', () => {
  beforeEach(() => { vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] }); seedDraft(); });
  afterEach(() => { vi.useRealTimers(); });

  it('reports saved and queued separately, then follows the receipt to completion and stops', async () => {
    const r = mount();
    await saveViaRecovery(r, receipt());
    expect(r.h.triage?.meeting).toMatchObject({ projectId: PROJECT_B.projectId, recordingId: 'rec-1', noteId: 'note-1' });
    expect(r.h.triage?.receipt?.status).toBe('pending'); // saved ≠ triaged

    getMeetingTriage
      .mockResolvedValueOnce(receipt({ status: 'running', sessionId: 'sess-tri' }))
      .mockResolvedValueOnce(receipt({ status: 'completed', sessionId: 'sess-tri', cardIds: ['c1', 'c2'] }));
    await advance(TRIAGE_POLL_DELAYS_MS[0]);
    expect(r.h.triage?.receipt?.status).toBe('running');
    await advance(TRIAGE_POLL_DELAYS_MS[1]);
    expect(r.h.triage?.receipt?.status).toBe('completed');
    expect(r.h.triage?.receipt?.cardIds).toEqual(['c1', 'c2']);
    expect(getMeetingTriage).toHaveBeenCalledWith(PROJECT_B.projectId, 'rec-1');

    await advance(10 * 60_000);
    expect(getMeetingTriage).toHaveBeenCalledTimes(2); // settled: no more polls
  });

  it('a failed triage can be retried, which resumes polling', async () => {
    const r = mount();
    await saveViaRecovery(r, receipt({ status: 'failed', lastError: 'model timeout', attemptCount: 1 }));
    await advance(60_000);
    expect(getMeetingTriage).not.toHaveBeenCalled(); // already settled

    retryMeetingTriage.mockResolvedValueOnce(receipt({ status: 'pending', attemptCount: 1 }));
    getMeetingTriage.mockResolvedValueOnce(receipt({ status: 'completed', cardIds: ['c9'] }));
    const ok = await run(() => r.h.retryTriage());
    expect(ok).toBe(true);
    expect(retryMeetingTriage).toHaveBeenCalledWith(PROJECT_B.projectId, 'rec-1');
    expect(r.h.triage?.receipt?.status).toBe('pending');
    await advance(TRIAGE_POLL_DELAYS_MS[0]);
    expect(r.h.triage?.receipt?.status).toBe('completed');
  });

  it('an older daemon with no receipt is reported as unavailable, never polled', async () => {
    const r = mount();
    await saveViaRecovery(r, undefined);
    expect(r.h.triage?.unavailable).toBe(true);
    await advance(60_000);
    expect(getMeetingTriage).not.toHaveBeenCalled();
  });

  it('gives up after repeated poll errors and offers a manual check', async () => {
    const r = mount();
    await saveViaRecovery(r, receipt());
    getMeetingTriage.mockRejectedValue(new Error('daemon down'));
    for (let i = 0; i < MAX_TRIAGE_POLL_ERRORS; i++) await advance(TRIAGE_POLL_DELAYS_MS[TRIAGE_POLL_DELAYS_MS.length - 1]);
    expect(r.h.triage?.stalled).toBe(true);
    expect(r.h.triage?.error).toBe('daemon down');
    const calls = getMeetingTriage.mock.calls.length;
    await advance(10 * 60_000);
    expect(getMeetingTriage).toHaveBeenCalledTimes(calls);

    getMeetingTriage.mockReset().mockResolvedValueOnce(receipt({ status: 'completed' }));
    await act(async () => { r.h.refreshTriage(); await vi.advanceTimersByTimeAsync(0); });
    expect(r.h.triage?.receipt?.status).toBe('completed');
    expect(r.h.triage?.stalled).toBe(false);
  });

  it('stops polling on unmount and picks the receipt back up on the next mount', async () => {
    const first = mount();
    await saveViaRecovery(first, receipt());
    first.unmount();
    await advance(10 * 60_000);
    expect(getMeetingTriage).not.toHaveBeenCalled();

    getMeetingTriage.mockResolvedValueOnce(receipt({ status: 'completed', cardIds: ['c1'] }));
    const second = mount();
    await advance(0);
    expect(getMeetingTriage).toHaveBeenCalledTimes(1);
    expect(second.h.triage?.receipt?.status).toBe('completed');
    expect(second.h.triage?.meeting.noteId).toBe('note-1');

    act(() => second.h.dismissTriage());
    second.unmount();
    getMeetingTriage.mockClear();
    const third = mount();
    await advance(0);
    expect(third.h.triage).toBeNull();
    expect(getMeetingTriage).not.toHaveBeenCalled();
  });

  it('a 404 marks the triage unavailable and stops', async () => {
    const r = mount();
    await saveViaRecovery(r, receipt());
    getMeetingTriage.mockRejectedValue(Object.assign(new Error('Meeting triage not found'), { status: 404 }));
    await advance(TRIAGE_POLL_DELAYS_MS[0]);
    expect(r.h.triage?.unavailable).toBe(true);
    await advance(10 * 60_000);
    expect(getMeetingTriage).toHaveBeenCalledTimes(1);
  });
});

// ── Source selection ────────────────────────────────────────────────────────

const PRACTICE_APP = { bundleId: 'ai.permagent.record-practice-20261007', name: 'Record Practice' };
const startArgs = () => invoke.mock.calls.find(c => c[0] === 'start_system_audio')?.[1] as Record<string, unknown> | undefined;

describe('sources', () => {
  it('records the microphone by default, as before', () => {
    const r = mount();
    expect(r.h.recordMic).toBe(true);
    expect(r.h.systemAudio).toBe(false);
    expect(r.h.systemAudioApp).toBeNull();
  });

  it('system audio only never opens the microphone, and labels the far side honestly', async () => {
    const r = mount();
    act(() => { r.h.setRecordMic(false); r.h.setSystemAudio(true); });
    await run(() => r.h.start(PROJECT_A));

    expect(navigator.mediaDevices.getUserMedia).not.toHaveBeenCalled();
    expect(invokeLog).toContain('start_system_audio');
    expect(r.h.state).toBe('recording');
    expect(r.h.activeSources).toEqual({ mic: false, system: true, app: null });

    farTexts.set('/tmp/sysaudio_0000.wav', 'agenda item one');
    act(() => emit('system_audio_chunk', '/tmp/sysaudio_0000.wav'));
    await run(() => r.h.stop());
    expect(noteBody()).toBe('**Others:** agenda item one');
    expect(noteBody()).not.toContain('**You:**');
    // The stable id still travels with a system-only save.
    expect(sentRecordingId()).toBeTruthy();
    expect(sentRecordingId()).toBe(createProjectNote.mock.calls[0][1].recordingId);
  });

  it('refuses to start with no source selected — nothing is opened', async () => {
    const r = mount();
    act(() => { r.h.setRecordMic(false); r.h.setSystemAudio(false); });
    await run(() => r.h.start(PROJECT_A));
    expect(r.h.state).toBe('error');
    expect(r.h.error).toMatch(/at least one source/);
    expect(navigator.mediaDevices.getUserMedia).not.toHaveBeenCalled();
    expect(invokeLog).not.toContain('start_system_audio');
    expect(readDrafts()).toEqual([]);
  });

  it('system audio only errors up front when the native has no capture helper', async () => {
    helperAvailable = false;
    const r = mount();
    act(() => { r.h.setRecordMic(false); r.h.setSystemAudio(true); });
    await run(() => r.h.start(PROJECT_A));
    expect(r.h.state).toBe('error');
    expect(r.h.error).toMatch(/isn't available in this build/);
    expect(navigator.mediaDevices.getUserMedia).not.toHaveBeenCalled();
    expect(invokeLog).not.toContain('start_system_audio');
    expect(r.h.target).toBeNull();
    expect(readDrafts()).toEqual([]);
  });

  it('system audio only: a helper start failure fails the start and cleans up', async () => {
    failStartSystemAudio = true;
    const r = mount();
    act(() => { r.h.setRecordMic(false); r.h.setSystemAudio(true); });
    await run(() => r.h.start(PROJECT_A));

    expect(r.h.state).toBe('error');
    expect(r.h.error).toMatch(/Couldn't start system audio capture: helper missing/);
    // Nothing is recording, so nothing may claim the mic carries on.
    expect(r.h.error).not.toMatch(/still being recorded/);
    expect(r.h.target).toBeNull();
    expect(r.h.activeSources).toBeNull();
    expect(handlers.system_audio_chunk ?? []).toHaveLength(0);
    expect(readDrafts()).toEqual([]);
    expect(navigator.mediaDevices.getUserMedia).not.toHaveBeenCalled();

    // A late event for the failed start files nothing.
    farTexts.set('/tmp/sysaudio_0000.wav', 'ghost');
    act(() => emit('system_audio_chunk', '/tmp/sysaudio_0000.wav'));
    await settle();
    expect(farReads()).toEqual([]);
  });

  it('passes the chosen app to the native, and only when one is chosen', async () => {
    const r = mount();
    act(() => { r.h.setSystemAudio(true); r.h.setSystemAudioApp(PRACTICE_APP); });
    await run(() => r.h.start(PROJECT_A));
    expect(startArgs()).toMatchObject({ includeAppBundleId: PRACTICE_APP.bundleId });
    expect(r.h.activeSources).toEqual({ mic: true, system: true, app: PRACTICE_APP });
    await run(() => r.h.stop());

    invoke.mockClear();
    act(() => r.h.setSystemAudioApp(null));
    await run(() => r.h.start(PROJECT_A));
    expect(startArgs()).toBeDefined();
    expect(startArgs()).not.toHaveProperty('includeAppBundleId'); // older native sees its old args
    r.unmount();
  });

  it('does not pass an app chosen while system audio is off', async () => {
    const r = mount();
    act(() => { r.h.setSystemAudioApp(PRACTICE_APP); });
    await run(() => r.h.start(PROJECT_A));
    expect(invokeLog).not.toContain('start_system_audio');
    expect(r.h.activeSources).toEqual({ mic: true, system: false, app: null });
    r.unmount();
  });

  it('lists capturable apps, and reports a native without the command as unable', async () => {
    const r = mount();
    invoke.mockImplementationOnce(async () => [
      { bundleId: 'b.zoom', name: 'Zoom' }, { bundleId: PRACTICE_APP.bundleId, name: PRACTICE_APP.name }, { name: 'no id' },
    ]);
    expect(await r.h.listSystemAudioApps()).toEqual([PRACTICE_APP, { bundleId: 'b.zoom', name: 'Zoom' }]);

    invoke.mockImplementationOnce(async () => { throw new Error('command list_system_audio_apps not found'); });
    expect(await r.h.listSystemAudioApps()).toBeNull();
  });
});

describe('chosen app is fail-closed', () => {
  it('refuses to start when the chosen app is no longer running — never widens to all audio', async () => {
    runningApps = [{ bundleId: 'us.zoom.xos', name: 'Zoom' }];
    const r = mount();
    act(() => { r.h.setSystemAudio(true); r.h.setSystemAudioApp(PRACTICE_APP); });
    await run(() => r.h.start(PROJECT_A));

    expect(r.h.state).toBe('error');
    expect(r.h.error).toBe('“Record Practice” isn\'t running, so it can\'t be recorded. Open it, or choose another source.');
    expect(r.h.systemAudioApp).toEqual(PRACTICE_APP); // retained, not reset to All
    expect(invokeLog).not.toContain('start_system_audio');
    expect(navigator.mediaDevices.getUserMedia).not.toHaveBeenCalled();
    expect(readDrafts()).toEqual([]);
  });

  it('refuses when this native cannot target an app at all', async () => {
    runningApps = 'missing';
    const r = mount();
    act(() => { r.h.setRecordMic(false); r.h.setSystemAudio(true); r.h.setSystemAudioApp(PRACTICE_APP); });
    await run(() => r.h.start(PROJECT_A));
    expect(r.h.state).toBe('error');
    expect(r.h.error).toMatch(/This build can't capture a single app/);
    expect(r.h.systemAudioApp).toEqual(PRACTICE_APP);
    expect(invokeLog).not.toContain('start_system_audio');
  });

  it('a native exact-target failure stays a failed start in system-only mode', async () => {
    failStartSystemAudio = true;
    const r = mount();
    act(() => { r.h.setRecordMic(false); r.h.setSystemAudio(true); r.h.setSystemAudioApp(PRACTICE_APP); });
    await run(() => r.h.start(PROJECT_A));
    expect(startArgs()).toMatchObject({ includeAppBundleId: PRACTICE_APP.bundleId });
    expect(r.h.state).toBe('error');
    expect(r.h.systemAudioApp).toEqual(PRACTICE_APP);
    expect(readDrafts()).toEqual([]);
  });
});

describe('helper error after Start', () => {
  async function systemOnlyRecording() {
    const r = mount();
    act(() => { r.h.setRecordMic(false); r.h.setSystemAudio(true); });
    await run(() => r.h.start(PROJECT_A));
    expect(r.h.state).toBe('recording');
    return r;
  }

  it('system only: ends the recording, stops the helper, keeps what was captured as a draft', async () => {
    const r = await systemOnlyRecording();
    farTexts.set('/tmp/sysaudio_0000.wav', 'first agenda item');
    act(() => emit('system_audio_chunk', '/tmp/sysaudio_0000.wav'));
    act(() => r.h.setUserNotes('follow up on budget'));
    invokeLog = [];

    await act(async () => {
      emit('system_audio_error', { kind: 'permission', detail: 'permission denied' });
      await new Promise(res => setTimeout(res, 0));
    });
    await settle();

    expect(r.h.state).toBe('error');
    expect(r.h.error).toMatch(/only source, so recording stopped/);
    expect(r.h.error).toMatch(/kept — save it from the recovered-recording card/);
    expect(invokeLog).toContain('stop_system_audio');
    expect(handlers.system_audio_chunk ?? []).toHaveLength(0);
    expect(handlers.system_audio_error ?? []).toHaveLength(0);
    expect(r.h.target).toBeNull();
    expect(r.h.activeSources).toBeNull();
    expect(createProjectNote).not.toHaveBeenCalled(); // never auto-saved

    // Recoverable: the far side AND the notepad, offered on the card.
    const [draft] = readDrafts();
    expect(draft.farParts).toEqual(['first agenda item']);
    expect(draft.userNotes).toBe('follow up on budget');
    expect(r.h.recoveredDrafts.map(d => d.startedAt)).toContain(draft.startedAt);

    // Closing the error panel does not delete it, and Stop is now a no-op.
    act(() => r.h.discard());
    expect(readDrafts()).toHaveLength(1);
    expect(await run(() => r.h.stop())).toBeNull();
    expect(createProjectNote).not.toHaveBeenCalled();
  });

  it('system only: with nothing captured yet, says so and leaves no empty draft', async () => {
    const r = await systemOnlyRecording();
    await act(async () => {
      emit('system_audio_error', { kind: 'capture', detail: 'stream ended' });
      await new Promise(res => setTimeout(res, 0));
    });
    await settle();
    expect(r.h.state).toBe('error');
    expect(r.h.error).toMatch(/System audio capture failed \(stream ended\), and it was the only source/);
    expect(r.h.error).toMatch(/Nothing had been captured yet/);
    expect(readDrafts()).toEqual([]);
    expect(r.h.recoveredDrafts).toEqual([]);
    expect(createProjectNote).not.toHaveBeenCalled();
  });

  it('mic + system: the mic keeps recording and the error names what was lost', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    invokeLog = [];
    act(() => emit('system_audio_error', { kind: 'capture', detail: 'stream ended' }));
    await settle();
    expect(r.h.state).toBe('recording');
    expect(r.h.systemAudioError).toBe('System audio capture failed (stream ended). Your microphone is still being recorded.');
    expect(invokeLog).not.toContain('stop_system_audio');
    expect(tracks[0].stopped).toBe(false);
    micTexts = ['still here'];
    speak(1);
    await run(() => r.h.stop());
    expect(noteBody()).toContain('still here');
    expect(noteBody()).toContain(GAP_MARKER);
    expect(r.h.failedChunks).toBe(1);
  });
});

describe('retry save ownership', () => {
  it('blocks a new recording while retry saves the old project transcript', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    micTexts = ['keep this in project A'];
    speak(1);
    createProjectNote.mockRejectedValueOnce(new Error('offline'));
    await run(() => r.h.stop());
    let release!: (value: {id: string}) => void;
    createProjectNote.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    let pending!: Promise<unknown>;
    act(() => { pending = r.h.retrySave(); });
    await run(() => r.h.start(PROJECT_B));
    expect(navigator.mediaDevices.getUserMedia).toHaveBeenCalledTimes(1);
    expect(r.h.state).toBe('finishing');
    await act(async () => { release({id: 'saved-a'}); await pending; });
    expect(createProjectNote.mock.calls[1][0]).toBe(PROJECT_A.projectId);
    expect(r.h.state).toBe('idle');
    expect(tracks.every(t => t.stopped)).toBe(true);
    expect(readDrafts()).toEqual([]);
    r.unmount();
  });
});

describe('transcription setup guard', () => {
  it('does not acquire either source or create a draft when the local model is unavailable', async () => {
    const r = mount();
    readiness.status = 'unavailable';
    readiness.message = 'No local transcription model is configured';
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    expect(r.h.state).toBe('error');
    expect(r.h.error).toBe(readiness.message);
    expect(navigator.mediaDevices.getUserMedia).not.toHaveBeenCalled();
    expect(invokeLog).not.toContain('start_system_audio');
    expect(readDrafts()).toEqual([]);
    r.unmount();
  });
});

describe('microphone context readiness', () => {
  it('resumes a suspended microphone context before claiming recording', async () => {
    contextInitiallySuspended = true;
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    expect(resumeAudioContext).toHaveBeenCalledTimes(1);
    expect(contexts[0].state).toBe('running');
    expect(r.h.state).toBe('recording');
    act(() => r.h.discard());
    await settle();
    r.unmount();
  });

  it('releases the microphone and reports a resume failure instead of recording silence', async () => {
    contextInitiallySuspended = true;
    resumeAudioContext.mockRejectedValueOnce(new Error('audio session unavailable'));
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    expect(r.h.state).toBe('error');
    expect(r.h.error).toContain('audio session unavailable');
    expect(tracks[0].stopped).toBe(true);
    expect(contexts[0].state).toBe('closed');
    expect(proc).toBeNull();
    expect(readDrafts()).toEqual([]);
    r.unmount();
  });
});

describe('capture and transcription failure evidence', () => {
  it('keeps a failed far-side write between its surrounding successful chunks in the saved note', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    transcribeAudio.mockResolvedValueOnce({text: 'before the missing segment'})
      .mockResolvedValueOnce({text: 'final commitment retained'});
    act(() => emit('system_audio_chunk', '/tmp/before.wav'));
    await settle();
    act(() => emit('system_audio_error', { kind: 'write', detail: 'disk write failed' }));
    act(() => emit('system_audio_chunk', '/tmp/after.wav'));
    await settle();
    expect(readDrafts()[0].farParts).toEqual(['before the missing segment', GAP_MARKER, 'final commitment retained']);
    await run(() => r.h.stop());
    expect(noteBody()).toBe(`**Others:** before the missing segment\n\n**Others:** ${GAP_MARKER}\n\n**Others:** final commitment retained`);
    expect(createProjectNote.mock.calls[0][0]).toBe(PROJECT_A.projectId);
    expect(r.h.failedChunks).toBe(1);
    r.unmount();
  });

  it('shows the local model setup failure instead of only an unexplained failed-segment count', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    transcribeAudio.mockRejectedValueOnce(new Error('No local dictation model is configured'));
    act(() => speak(MEETING_CHUNK_SECONDS));
    await settle();
    expect(r.h.error).toMatch(/No local dictation model is configured/);
    expect(r.h.error).toMatch(/Stop and resolve the local transcription error/);
    expect(r.h.failedChunks).toBe(1);
    expect(readDrafts()[0].parts).toEqual([GAP_MARKER]);
    act(() => r.h.discard());
    await settle();
    r.unmount();
  });
});

describe('retired recording and final capture errors', () => {
  it('does not claim the far side was heard when the received chunk is silent', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    act(() => emit('system_audio_chunk', '/tmp/silent.wav'));
    await settle();
    expect(r.h.farChunksHeard).toBe(0);
    farTexts.set('/tmp/speech.wav', 'actual far side words');
    act(() => emit('system_audio_chunk', '/tmp/speech.wav'));
    await settle();
    expect(r.h.farChunksHeard).toBe(1);
    act(() => r.h.discard());
    await settle();
    r.unmount();
  });

  it('retires pending microphone transcription before another project starts', async () => {
    const r = mount();
    await run(() => r.h.start(PROJECT_A));
    let release!: (value: { text: string }) => void;
    transcribeAudio.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    speak(MEETING_CHUNK_SECONDS);
    await settle();
    act(() => r.h.discard());
    await settle();
    await run(() => r.h.start(PROJECT_B));
    await act(async () => { release({ text: 'private words from discarded project A' }); });
    await settle();
    expect(readDrafts().find(d => d.projectId === PROJECT_B.projectId)?.parts).toEqual([]);
    expect(r.h.nearChunksHeard).toBe(0);
    micTexts = ['only fresh project B words'];
    speak(1);
    await run(() => r.h.stop());
    expect(noteBody()).toBe('only fresh project B words');
    expect(createProjectNote.mock.calls[0][0]).toBe(PROJECT_B.projectId);
    r.unmount();
  });

  it('closes a late microphone stream when unmounted during permission resolution', async () => {
    const r = mount();
    let release!: (value: { getTracks: () => FakeTrack[] }) => void;
    vi.mocked(navigator.mediaDevices.getUserMedia).mockImplementationOnce(() => new Promise(resolve => {
      release = value => resolve(value as unknown as MediaStream);
    }));
    let starting!: Promise<void>;
    act(() => { starting = r.h.start(PROJECT_A); });
    await settle();
    r.unmount();
    const lateTrack = new FakeTrack();
    try {
      await act(async () => { release({ getTracks: () => [lateTrack] }); await starting; });
      expect(lateTrack.stopped).toBe(true);
      expect(contexts.every(context => context.state === 'closed')).toBe(true);
      expect(readDrafts()).toEqual([]);
    } finally {
      act(() => r.h.discard());
    }
  });

  it('takes a failed-stop receipt tail once, freezes it for retry, and ignores its late duplicate event', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    const tail = '/tmp/final-error-tail.wav';
    farTexts.set(tail, 'Final decision: do not purchase. Review the internal checklist.');
    const lateEvent = handlers.system_audio_chunk[0];
    stopReturns = [{unreadPaths: [tail], error: 'capture helper exited unsuccessfully (exit status: 7)'}];
    const saved = await run(() => r.h.stop());
    expect(saved).toBeNull();
    expect(createProjectNote).not.toHaveBeenCalled();
    expect(r.h.state).toBe('error');
    expect(farReads()).toEqual([tail]);
    const draft = readDrafts()[0];
    expect(draft.body).toContain('Final decision: do not purchase. Review the internal checklist.');
    expect(draft.body).toContain(GAP_MARKER);
    act(() => lateEvent({payload: tail}));
    await settle();
    expect(farReads()).toEqual([tail]);
    await run(() => r.h.retrySave());
    expect(sentRecordingId()).toBe(draft.recordingId);
    expect(noteBody()).toBe(draft.body);
    expect(createProjectNote.mock.calls[0][0]).toBe(PROJECT_A.projectId);
    r.unmount();
  });

  it('keeps a recoverable draft when the final system audio flush fails', async () => {
    const r = mount();
    act(() => r.h.setSystemAudio(true));
    await run(() => r.h.start(PROJECT_A));
    act(() => r.h.setUserNotes('keep the typed commitment'));
    micTexts = ['confirmed before stop'];
    speak(1);
    invoke.mockImplementationOnce(async (command: string) => {
      expect(command).toBe('stop_system_audio');
      throw new Error('capture helper timed out; final audio may be incomplete');
    });
    const saved = await run(() => r.h.stop());
    expect(saved).toBeNull();
    expect(createProjectNote).not.toHaveBeenCalled();
    expect(r.h.state).toBe('error');
    expect(r.h.error).toMatch(/final audio|incomplete/i);
    expect(r.h.hasUnsavedTranscript).toBe(true);
    const draft = readDrafts()[0];
    expect(draft.body).toContain('confirmed before stop');
    expect(draft.body).toContain('keep the typed commitment');
    expect(draft.body).toContain(GAP_MARKER);
    const stableId = draft.recordingId;
    await run(() => r.h.retrySave());
    expect(sentRecordingId()).toBe(stableId);
    expect(noteBody()).toBe(draft.body);
    r.unmount();
  });
});
