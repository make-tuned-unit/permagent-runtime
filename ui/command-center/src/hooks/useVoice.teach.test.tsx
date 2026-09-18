/**
 * @vitest-environment jsdom
 *
 * useVoice — pronunciation-teach drill (2026-09-01 incident, FIX 1/3a/4 client
 * halves).
 *
 * Three symptoms, traced at the hook level with a fully controllable mock
 * WebSocket + AudioContext:
 *
 *   FIX 1 — the `teach` frame used to be applied the instant it arrived
 *   (synthesis-QUEUE time server-side), well before its own ASK_FIRST/
 *   ASK_AGAIN audio reached the speaker. Deferred behind the audio queue,
 *   mirroring pendingNavRef/flushNavIfIdle.
 *
 *   FIX 3a — the 'idle' handler re-armed recording unconditionally whenever
 *   teachWordRef was set, even for a zero-sample capture. Capped at
 *   MAX_TEACH_ZERO_SAMPLE_LISTENS (3), then the client gives up locally and
 *   tells the server (`teach_skip`) to do the same SKIPPED + resume a spoken
 *   "skip" would.
 *
 *   FIX 4 (client half) — deactivate() never reset teachWord, so the pill
 *   could survive "click anywhere to end".
 */

import { describe, it, expect, vi, beforeEach, afterEach, type MockInstance } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { loadDaemonToken } = vi.hoisted(() => ({
  loadDaemonToken: vi.fn(() => Promise.resolve('tok-test')),
}));

vi.mock('../lib/api', () => ({
  getApiBaseUrl: () => 'http://127.0.0.1:3001',
  loadDaemonToken,
}));

import { useVoice } from './useVoice';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// ── WebSocket mock ──────────────────────────────────────────────────────────

class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  static instances: MockWebSocket[] = [];

  url: string;
  binaryType = 'blob';
  readyState = MockWebSocket.OPEN;
  closed = false;
  sent: unknown[] = [];
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((ev: { code: number; reason: string; wasClean: boolean }) => void) | null = null;

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  send(data: unknown) {
    this.sent.push(data);
  }

  close() {
    this.closed = true;
    this.readyState = MockWebSocket.CLOSED;
  }

  serverReady() {
    this.onmessage?.({ data: JSON.stringify({ type: 'ready' }) });
  }

  /** Send a JSON server frame (teach/idle/reply_end/transcript/taught/...). */
  frame(msg: Record<string, unknown>) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }

  /** Send a binary reply-audio chunk. */
  audioChunk(samples = new Float32Array([0.1, 0.2, 0.3])) {
    this.onmessage?.({ data: samples.buffer });
  }

  jsonSent(): Array<Record<string, unknown>> {
    return this.sent
      .filter((d): d is string => typeof d === 'string')
      .map(d => JSON.parse(d) as Record<string, unknown>);
  }
}

// ── Audio mocks ──────────────────────────────────────────────────────────
//
// createBufferSource returns an object this test controls directly — calling
// `.onended?.()` simulates one queued chunk finishing playback, exactly like
// the real Web Audio API does asynchronously. This is what lets a test hold
// "audio is still playing" open for as long as it wants, to prove a `teach`
// frame arriving mid-playback is NOT applied early.

interface FakeBufferSource {
  buffer: unknown;
  onended: (() => void) | null;
  connect: ReturnType<typeof vi.fn>;
  start: ReturnType<typeof vi.fn>;
}

let bufferSources: FakeBufferSource[] = [];

class MockAudioContext {
  state = 'running';
  destination = {};
  createMediaStreamSource() {
    return { connect: vi.fn() };
  }
  createScriptProcessor() {
    return { connect: vi.fn(), disconnect: vi.fn(), onaudioprocess: null };
  }
  createAnalyser() {
    return { connect: vi.fn(), fftSize: 0, smoothingTimeConstant: 0 };
  }
  createBuffer(_channels: number, length: number, _sampleRate: number) {
    return { length, getChannelData: () => new Float32Array(length) };
  }
  createBufferSource(): FakeBufferSource {
    const src: FakeBufferSource = {
      buffer: null,
      onended: null,
      connect: vi.fn(),
      start: vi.fn(),
    };
    bufferSources.push(src);
    return src;
  }
  close() {
    this.state = 'closed';
    return Promise.resolve();
  }
}

const micTrack = { stop: vi.fn() };
const micStream = { getTracks: () => [micTrack] };

// ── Hook host ───────────────────────────────────────────────────────────────

type VoiceApi = ReturnType<typeof useVoice>;
const hook: { current: VoiceApi | null } = { current: null };

function Host({ sessionId }: { sessionId?: string }) {
  hook.current = useVoice({ sessionId });
  return null;
}

let container: HTMLDivElement;
let root: Root;
let warnSpy: MockInstance;

function render(sessionId?: string) {
  act(() => root.render(<Host sessionId={sessionId} />));
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function activateTo(sessionId: string): Promise<MockWebSocket> {
  render(sessionId);
  act(() => { void hook.current!.activate(); });
  await flush();
  const ws = MockWebSocket.instances[MockWebSocket.instances.length - 1];
  act(() => ws.serverReady());
  await flush();
  expect(hook.current!.state).toBe('ready');
  return ws;
}

beforeEach(() => {
  MockWebSocket.instances = [];
  bufferSources = [];
  hook.current = null;
  loadDaemonToken.mockImplementation(() => Promise.resolve('tok-test'));
  vi.stubGlobal('WebSocket', MockWebSocket);
  vi.stubGlobal('AudioContext', MockAudioContext);
  Object.defineProperty(navigator, 'mediaDevices', {
    configurable: true,
    value: { getUserMedia: vi.fn(() => Promise.resolve(micStream)) },
  });
  warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('FIX 1 — teach frame deferred behind the audio queue', () => {
  it('a teach frame arriving mid-playback does NOT set teachWord until the queue actually drains', async () => {
    const ws = await activateTo('session-A');

    // ASK_FIRST's audio starts streaming (the reply's own narration).
    act(() => ws.audioChunk());
    await flush();
    expect(bufferSources).toHaveLength(1);
    expect(hook.current!.state).toBe('playing');

    // The server sends `teach` at synthesis-QUEUE time — well before this
    // audio (or ASK_FIRST's own audio, which hasn't even synthesized yet)
    // finishes. This is the literal bug: teachWord must NOT be set yet.
    act(() => ws.frame({ type: 'teach', word: 'Elspeth' }));
    expect(hook.current!.teachWord).toBeNull();
    expect(ws.jsonSent().some(m => m.type === 'start')).toBe(false);

    // The chunk finishes playing; nothing else is queued yet but the reply
    // hasn't ended (pendingAudioRef still true) — still must not apply.
    act(() => bufferSources[0].onended?.());
    await flush();
    expect(hook.current!.teachWord).toBeNull();

    // Only once the server signals the reply is fully drained does the pill
    // apply — and the recording it arms starts in the very same tick.
    act(() => ws.frame({ type: 'reply_end', sample_rate: 24000 }));
    await flush();
    expect(hook.current!.teachWord).toBe('Elspeth');
    expect(ws.jsonSent().some(m => m.type === 'start')).toBe(true);
    expect(hook.current!.state).toBe('recording');
  });

  it('applies immediately when the queue is already idle when the frame arrives', async () => {
    const ws = await activateTo('session-B');
    act(() => ws.frame({ type: 'teach', word: 'Taran' }));
    expect(hook.current!.teachWord).toBe('Taran');
  });
});

describe('FIX 3a — bounded consecutive zero-sample listens', () => {
  it('caps at MAX_TEACH_ZERO_SAMPLE_LISTENS=3: at most 2 re-arms, then the drill is dropped and teach_skip is sent', async () => {
    const ws = await activateTo('session-C');
    act(() => ws.frame({ type: 'teach', word: 'Elspeth' }));
    expect(hook.current!.teachWord).toBe('Elspeth');

    // Every `idle` here simulates a listen that captured 0 samples — the
    // server's actual response to a too-short buffer (voice.rs: min_samples).
    act(() => ws.frame({ type: 'idle' }));
    expect(ws.jsonSent().filter(m => m.type === 'start')).toHaveLength(1);
    expect(hook.current!.teachWord).toBe('Elspeth');

    act(() => ws.frame({ type: 'idle' }));
    expect(ws.jsonSent().filter(m => m.type === 'start')).toHaveLength(2);
    expect(hook.current!.teachWord).toBe('Elspeth');

    // Third zero-sample listen in a row — the cap trips: no third restart,
    // the pill clears locally, and the server is told to do the same
    // SKIPPED + resume a spoken "skip" would.
    act(() => ws.frame({ type: 'idle' }));
    expect(ws.jsonSent().filter(m => m.type === 'start')).toHaveLength(2);
    expect(hook.current!.teachWord).toBeNull();
    expect(ws.jsonSent().filter(m => m.type === 'teach_skip')).toHaveLength(1);
  });

  it('a real transcript resets the budget — only ZERO-sample listens count against the cap', async () => {
    const ws = await activateTo('session-D');
    act(() => ws.frame({ type: 'teach', word: 'Barty' }));

    act(() => ws.frame({ type: 'idle' }));
    act(() => ws.frame({ type: 'idle' }));
    expect(ws.jsonSent().filter(m => m.type === 'start')).toHaveLength(2);

    // A real capture lands — STT actually ran. The budget resets.
    act(() => ws.frame({ type: 'transcript', text: 'bar tea' }));

    act(() => ws.frame({ type: 'idle' }));
    act(() => ws.frame({ type: 'idle' }));
    // Would have been capped at the 3rd zero-sample listen without the
    // reset (2 already spent); with the reset, both of these succeed.
    expect(ws.jsonSent().filter(m => m.type === 'start')).toHaveLength(4);
    expect(hook.current!.teachWord).toBe('Barty');
    expect(ws.jsonSent().filter(m => m.type === 'teach_skip')).toHaveLength(0);
  });
});

describe('FIX 4 (client half) — deactivate() clears the pill', () => {
  it('deactivate() resets teachWord even mid-drill', async () => {
    await activateTo('session-E');
    const ws = MockWebSocket.instances[MockWebSocket.instances.length - 1];
    act(() => ws.frame({ type: 'teach', word: 'Prideine' }));
    expect(hook.current!.teachWord).toBe('Prideine');

    act(() => hook.current!.deactivate());

    expect(hook.current!.teachWord).toBeNull();
    expect(hook.current!.state).toBe('idle');
  });
});

describe('Happy path (GATES): teach → ASK_FIRST audible → pill+record together → transcript → taught → resume', () => {
  it('traces the full listen-once flow without ever restarting after taught', async () => {
    const ws = await activateTo('session-F');

    // 1. Model hits an unknown name mid-reply; server queues `teach` +
    //    ASK_FIRST's audio (still streaming).
    act(() => ws.audioChunk());
    act(() => ws.frame({ type: 'teach', word: 'Elspeth' }));
    expect(hook.current!.teachWord).toBeNull(); // not yet — audio still playing

    // 2. ASK_FIRST finishes playing and the reply ends — pill + recording
    //    land together.
    act(() => bufferSources[0].onended?.());
    act(() => ws.frame({ type: 'reply_end', sample_rate: 24000 }));
    await flush();
    expect(hook.current!.teachWord).toBe('Elspeth');
    expect(hook.current!.state).toBe('recording');
    const startsAfterAsk = ws.jsonSent().filter(m => m.type === 'start').length;
    expect(startsAfterAsk).toBe(1);

    // 3. User answers; a real transcript comes back (not zero-sample).
    act(() => ws.frame({ type: 'transcript', text: "it's like else peth" }));
    expect(hook.current!.lastTranscript).toBe("it's like else peth");

    // 4. Server saved it — Taught clears the pill immediately (not deferred;
    //    only the initial `teach` needed the audio-queue defer).
    act(() => ws.frame({ type: 'taught', word: 'Elspeth' }));
    expect(hook.current!.teachWord).toBeNull();

    // 5. The confirmation ("Got it...") plays, then the held story resumes.
    act(() => ws.frame({ type: 'reply_start' }));
    act(() => ws.audioChunk());
    act(() => bufferSources[bufferSources.length - 1].onended?.());
    act(() => ws.frame({ type: 'reply_end', sample_rate: 24000 }));
    await flush();

    // No further restart — teachWord stayed null through the whole resume.
    expect(hook.current!.teachWord).toBeNull();
    expect(ws.jsonSent().filter(m => m.type === 'start')).toHaveLength(startsAfterAsk);
    expect(hook.current!.state).toBe('ready');
    // Never fell back to the zero-sample give-up path.
    expect(warnSpy).not.toHaveBeenCalled();
  });
});
