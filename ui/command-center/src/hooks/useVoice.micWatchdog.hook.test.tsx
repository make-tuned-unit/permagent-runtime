/**
 * @vitest-environment jsdom
 *
 * useVoice — the mic-input watchdog, wired end to end through the hook.
 *
 * User report (2026-09-18): voice "isn't responding and just gets stuck on
 * listening". Five sessions in the daemon log where the wake/VAD gate never
 * opened a turn. The hands-free VAD graph had no ctx.resume(), no statechange
 * / devicechange / track listeners, and startVadMonitor returned early while a
 * (dead) context existed — so a suspended graph stayed dead and the orb said
 * "Listening" until the user closed it.
 *
 * What this proves: with a mock AudioContext whose processor NEVER fires (the
 * deaf graph), the hook rebuilds the graph once, then leaves 'ready' for
 * 'error' with "Not hearing the microphone — check the input device"; a
 * buffer on the rebuilt graph brings it back to 'ready'; a healthy graph is
 * left alone; devicechange re-acquires the mic; a suspended context is
 * resumed; a VAD recording on a deaf graph is closed instead of hanging.
 * Also: buffers that keep ARRIVING but are all zeros (HFP renegotiation) are a
 * fault too; and a live mic behind an armed wake gate whose spotter ignores
 * repeated speech fails the gate open instead of asking for "Hey Henry"
 * forever — unless the spotter has fired this session. At the forensic HFP
 * level (0.015 RMS, between keepalive and onset) the failed-open gate really
 * lets speech open a turn; the verdict survives a reconnect's fresh
 * wake_status; a device change or switching the wake word off clears it.
 *
 * What it does NOT prove: that WKWebView really suspends/fires these events
 * on an A2DP<->HFP flip — jsdom has no audio stack. That needs the real app.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { loadDaemonToken } = vi.hoisted(() => ({
  loadDaemonToken: vi.fn(() => Promise.resolve('tok-test')),
}));

vi.mock('../lib/api', () => ({
  getApiBaseUrl: () => 'http://127.0.0.1:3001',
  loadDaemonToken,
}));

import {
  MIC_FAULT_MESSAGE,
  MIC_SILENT_MESSAGE,
  MIC_STALL_MS,
  MIC_WATCHDOG_TICK_MS,
  WAKE_DEAF_GATED_MS,
  WAKE_REARM_MS,
  VOICE_VERDICT_KEY,
  VOICE_VERDICT_LOG_MAX,
  readVoiceVerdict,
  readVoiceVerdictLog,
  recordVoiceVerdictEvent,
  useVoice,
} from './useVoice';

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
  sent: unknown[] = [];
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((ev: { code: number; reason: string; wasClean: boolean }) => void) | null = null;
  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }
  send(data: unknown) { this.sent.push(data); }
  close() { this.readyState = MockWebSocket.CLOSED; }
  frame(msg: object) { this.onmessage?.({ data: JSON.stringify(msg) }); }
  jsonSent(): Array<{ type: string }> {
    return this.sent.filter((d): d is string => typeof d === 'string').map(d => JSON.parse(d));
  }
}

// ── Audio mocks ─────────────────────────────────────────────────────────────

interface MockProc {
  bufferSize: number;
  connect: ReturnType<typeof vi.fn>;
  disconnect: ReturnType<typeof vi.fn>;
  onaudioprocess: ((e: unknown) => void) | null;
}

class MockAudioContext {
  static instances: MockAudioContext[] = [];
  state = 'running';
  destination = {};
  procs: MockProc[] = [];
  onstatechange: (() => void) | null = null;
  resume = vi.fn(() => { this.state = 'running'; return Promise.resolve(); });
  constructor() { MockAudioContext.instances.push(this); }
  createMediaStreamSource() { return { connect: vi.fn() }; }
  createScriptProcessor(bufferSize: number) {
    const p: MockProc = { bufferSize, connect: vi.fn(), disconnect: vi.fn(), onaudioprocess: null };
    this.procs.push(p);
    return p;
  }
  createAnalyser() {
    return { connect: vi.fn(), fftSize: 0, smoothingTimeConstant: 0, frequencyBinCount: 32, getByteFrequencyData: vi.fn() };
  }
  close() { this.state = 'closed'; return Promise.resolve(); }
}

/** The VAD monitor's processor (2048) on the newest context that has one. */
function vadProc(): MockProc {
  for (let i = MockAudioContext.instances.length - 1; i >= 0; i--) {
    const p = MockAudioContext.instances[i].procs.find(pp => pp.bufferSize === 2048);
    if (p) return p;
  }
  throw new Error('no VAD processor');
}
function vadContexts() {
  return MockAudioContext.instances.filter(c => c.procs.some(p => p.bufferSize === 2048));
}

/** Room tone: well above digital silence, well below any VAD threshold. */
const ROOM = 0.002;
/** Speech-level buffer: over VAD onset (0.025) and keepalive (0.010). */
const VOICE = 0.05;
/** The 2026-09-01 forensic level: a Bose QC35 on HFP delivering DSP-flattened
 *  speech ABOVE keepalive (0.010) but BELOW onset (0.025). */
const HFP_VOICE = 0.015;

/** Feed one buffer through the VAD processor. Default is room tone — a live,
 *  quiet mic. Pass 0 for the digital-silence case. */
function pump(level = ROOM) {
  const data = new Float32Array(2048).fill(level);
  vadProc().onaudioprocess?.({ inputBuffer: { getChannelData: () => data } });
}

function makeStream() {
  const track: Record<string, unknown> = { stop: vi.fn(), readyState: 'live', muted: false };
  return { track, stream: { getTracks: () => [track] } };
}

let deviceListeners: Array<() => void> = [];
type FakeStream = ReturnType<typeof makeStream>['stream'];
let getUserMedia: ReturnType<typeof vi.fn<[], Promise<FakeStream>>>;

// ── Host ───────────────────────────────────────────────────────────────────

type VoiceApi = ReturnType<typeof useVoice>;
const hook: { current: VoiceApi | null } = { current: null };
function Host() {
  hook.current = useVoice({ sessionId: 's1' });
  return null;
}

let container: HTMLDivElement;
let root: Root;

async function flush() {
  await act(async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
  });
}

async function advance(ms: number) {
  await act(async () => { vi.advanceTimersByTime(ms); });
  await flush();
}

/** Hands-free on and the socket ready. No buffer is pumped: tests that want a live graph pump it themselves. */
async function armHandsFree(): Promise<MockWebSocket> {
  act(() => root.render(<Host />));
  act(() => { void hook.current!.setHandsFree(true); });
  await flush();
  const ws = MockWebSocket.instances[0];
  act(() => ws.frame({ type: 'ready' }));
  await flush();
  expect(hook.current!.state).toBe('ready');
  expect(vadContexts()).toHaveLength(1);
  return ws;
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
  MockWebSocket.instances = [];
  MockAudioContext.instances = [];
  deviceListeners = [];
  hook.current = null;
  vi.stubGlobal('WebSocket', MockWebSocket);
  vi.stubGlobal('AudioContext', MockAudioContext);
  getUserMedia = vi.fn<[], Promise<FakeStream>>(() => Promise.resolve(makeStream().stream));
  Object.defineProperty(navigator, 'mediaDevices', {
    configurable: true,
    value: {
      getUserMedia,
      addEventListener: (t: string, fn: () => void) => { if (t === 'devicechange') deviceListeners.push(fn); },
      removeEventListener: (t: string, fn: () => void) => {
        if (t === 'devicechange') deviceListeners = deviceListeners.filter(f => f !== fn);
      },
    },
  });
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('useVoice — mic-input watchdog', () => {
  it('a deaf graph is rebuilt once, then the orb leaves "Listening" for a clear mic fault', async () => {
    await armHandsFree();
    // The processor never fires: the Bluetooth-flip / sleep-wake case.
    await advance(MIC_STALL_MS + MIC_WATCHDOG_TICK_MS);
    expect(vadContexts()).toHaveLength(2); // one rebuild
    expect(hook.current!.state).toBe('ready'); // not surfaced yet

    // The rebuilt graph is dead too.
    await advance(MIC_STALL_MS + MIC_WATCHDOG_TICK_MS);
    expect(hook.current!.state).toBe('error');
    expect(hook.current!.error).toBe(MIC_FAULT_MESSAGE);

    // Audio comes back on the rebuilt graph → back to listening, fault cleared.
    act(() => pump());
    await advance(MIC_WATCHDOG_TICK_MS);
    expect(hook.current!.state).toBe('ready');
    expect(hook.current!.error).toBeNull();
  });

  it('leaves a healthy graph alone', async () => {
    await armHandsFree();
    for (let i = 0; i < 20; i++) {
      act(() => pump());
      await advance(500);
    }
    expect(vadContexts()).toHaveLength(1);
    expect(hook.current!.state).toBe('ready');
    expect(hook.current!.error).toBeNull();
  });

  it('also watches a wake-gated "ready" — the gate can no longer hide a dead mic', async () => {
    const ws = await armHandsFree();
    act(() => ws.frame({ type: 'wake_status', active: true, phrase: 'Hey Henry' }));
    await flush();
    expect(hook.current!.wakeWord.gated).toBe(true);
    await advance(2 * (MIC_STALL_MS + MIC_WATCHDOG_TICK_MS));
    expect(hook.current!.state).toBe('error');
    expect(hook.current!.error).toBe(MIC_FAULT_MESSAGE);
  });

  it('resumes a suspended VAD context on statechange', async () => {
    await armHandsFree();
    const ctx = vadContexts()[0];
    ctx.state = 'suspended';
    act(() => ctx.onstatechange?.());
    expect(ctx.resume).toHaveBeenCalled();
  });

  it('re-acquires the mic and rebuilds the graph when input devices change', async () => {
    await armHandsFree();
    expect(getUserMedia).toHaveBeenCalledTimes(1);
    expect(deviceListeners).toHaveLength(1);
    act(() => deviceListeners.forEach(fn => fn()));
    await advance(MIC_WATCHDOG_TICK_MS);
    expect(getUserMedia).toHaveBeenCalledTimes(2);
    expect(vadContexts()).toHaveLength(2);
  });

  it('re-acquires when the mic track ends', async () => {
    act(() => root.render(<Host />));
    const first = makeStream();
    getUserMedia.mockImplementationOnce(() => Promise.resolve(first.stream));
    act(() => { void hook.current!.setHandsFree(true); });
    await flush();
    act(() => MockWebSocket.instances[0].frame({ type: 'ready' }));
    await flush();
    first.track.readyState = 'ended';
    act(() => (first.track.onended as () => void)());
    await advance(MIC_WATCHDOG_TICK_MS);
    expect(getUserMedia).toHaveBeenCalledTimes(2);
    expect(first.track.stop).toHaveBeenCalled();
  });

  it('closes a VAD recording whose graph went deaf instead of listening forever', async () => {
    const ws = await armHandsFree();
    act(() => hook.current!.startRecording());
    expect(hook.current!.state).toBe('recording');
    await advance(MIC_STALL_MS + MIC_WATCHDOG_TICK_MS);
    expect(ws.jsonSent().some(m => m.type === 'stop')).toBe(true);
    expect(hook.current!.state).toBe('processing');
  });

  it('buffers that keep arriving but are all zeros are a fault, not "hearing you"', async () => {
    await armHandsFree();
    // ~15s of digital silence, pumped every 250ms on whichever graph is live.
    for (let i = 0; i < 60; i++) {
      act(() => pump(0));
      await advance(250);
    }
    // Re-acquired once (a fresh getUserMedia is the plausible fix), then surfaced.
    expect(getUserMedia).toHaveBeenCalledTimes(2);
    expect(hook.current!.state).toBe('error');
    expect(hook.current!.error).toBe(MIC_SILENT_MESSAGE);

    // Signal returns → back to listening.
    act(() => pump());
    await advance(MIC_WATCHDOG_TICK_MS);
    expect(hook.current!.state).toBe('ready');
    expect(hook.current!.error).toBeNull();
  });

  it('a live mic behind a wake gate whose spotter ignores repeated speech fails open', async () => {
    const ws = await armHandsFree();
    act(() => ws.frame({ type: 'wake_status', active: true, phrase: 'Hey Henry' }));
    await flush();
    expect(hook.current!.wakeWord.gated).toBe(true);

    // Say it three times, seconds apart; the spotter never answers.
    const speakAt = new Set([2, 6, 10]);
    for (let sec = 0; sec < 14; sec++) {
      for (let b = 0; b < 4; b++) act(() => pump(speakAt.has(sec) ? VOICE : ROOM));
      await advance(1_000);
    }
    // Enough episodes, not yet enough time: still gated, no turn opened.
    expect(hook.current!.wakeWord.gated).toBe(true);
    expect(hook.current!.wakeWord.spotterDeaf).toBe(false);
    expect(hook.current!.state).toBe('ready');

    await advance(WAKE_DEAF_GATED_MS - 14_000);
    act(() => pump());
    await flush();
    expect(hook.current!.wakeWord.gated).toBe(false);
    expect(hook.current!.wakeWord.spotterDeaf).toBe(true);
    expect(hook.current!.state).toBe('ready');

    // Bare speech now opens a turn.
    act(() => { pump(VOICE); pump(VOICE); });
    await flush();
    expect(hook.current!.state).toBe('recording');
  });

  /** Arm the wake gate, say it three times at `level` with no spotter answer,
   *  and wait out WAKE_DEAF_GATED_MS until the gate fails open. */
  async function failOpenAt(ws: MockWebSocket, level: number) {
    act(() => ws.frame({ type: 'wake_status', active: true, phrase: 'Hey Henry' }));
    await flush();
    expect(hook.current!.wakeWord.gated).toBe(true);
    const speakAt = new Set([2, 6, 10]);
    const secs = Math.ceil(WAKE_DEAF_GATED_MS / 1_000) + 1;
    for (let sec = 0; sec < secs; sec++) {
      for (let b = 0; b < 4; b++) act(() => pump(speakAt.has(sec) ? level : ROOM));
      await advance(1_000);
    }
    expect(hook.current!.wakeWord.gated).toBe(false);
    expect(hook.current!.wakeWord.spotterDeaf).toBe(true);
    expect(hook.current!.state).toBe('ready');
  }

  it('at the forensic HFP level (between keepalive and onset), "just talk" is true: speech opens a turn', async () => {
    // Before: episodes were counted at keepalive, so the gate failed open and
    // the orb said "just talk" — but onset still demanded 0.025, so the very
    // speech that earned the verdict could never open a turn.
    const ws = await armHandsFree();
    await failOpenAt(ws, HFP_VOICE);
    act(() => { pump(HFP_VOICE); pump(HFP_VOICE); });
    await flush();
    expect(hook.current!.state).toBe('recording');
  });

  it('room tone still opens no turn after the verdict — the lowered bar is not "any sound"', async () => {
    const ws = await armHandsFree();
    await failOpenAt(ws, HFP_VOICE);
    for (let i = 0; i < 20; i++) act(() => pump(ROOM));
    await flush();
    expect(hook.current!.state).toBe('ready');
  });

  it('a reconnect (fresh wake_status) after the idle window does not re-trap a deaf verdict behind the gate', async () => {
    // The daemon sends wake_status on every socket (re)connect, and a
    // barge-in tears the socket down. Resetting the verdict there cost the
    // user another 15s and three ignored attempts after every reconnect.
    const ws = await armHandsFree();
    await failOpenAt(ws, HFP_VOICE);
    const idleSecs = Math.ceil(WAKE_REARM_MS / 1_000) + 5;
    for (let sec = 0; sec < idleSecs; sec++) {
      for (let b = 0; b < 4; b++) act(() => pump(ROOM));
      await advance(1_000);
    }
    act(() => ws.frame({ type: 'wake_status', active: true, phrase: 'Hey Henry' }));
    await flush();
    expect(hook.current!.wakeWord.spotterDeaf).toBe(true);
    expect(hook.current!.wakeWord.gated).toBe(false);
    act(() => { pump(HFP_VOICE); pump(HFP_VOICE); });
    await flush();
    expect(hook.current!.state).toBe('recording');
  });

  it('a new input device clears the deaf verdict — the new mic gets a working wake gate again', async () => {
    const ws = await armHandsFree();
    await failOpenAt(ws, HFP_VOICE);
    act(() => deviceListeners.forEach(fn => fn()));
    await flush();
    expect(hook.current!.wakeWord.spotterDeaf).toBe(false);
  });

  it('switching the wake word off clears the deaf hint', async () => {
    const ws = await armHandsFree();
    await failOpenAt(ws, HFP_VOICE);
    act(() => hook.current!.wakeWord.setEnabled(false));
    await flush();
    expect(hook.current!.wakeWord.spotterDeaf).toBe(false);
  });

  it('never fails the gate open once the spotter has fired this session', async () => {
    const ws = await armHandsFree();
    act(() => ws.frame({ type: 'wake_status', active: true, phrase: 'Hey Henry' }));
    await flush();
    // A 'stop' detection proves the spotter hears, without opening a turn.
    act(() => ws.frame({ type: 'wake', kind: 'stop' }));
    await flush();
    for (let sec = 0; sec < 30; sec++) {
      for (let b = 0; b < 4; b++) act(() => pump(sec % 3 === 0 ? VOICE : ROOM));
      await advance(1_000);
    }
    expect(hook.current!.wakeWord.gated).toBe(true);
    expect(hook.current!.wakeWord.spotterDeaf).toBe(false);
    expect(hook.current!.state).toBe('ready');
  });

  it('leaving hands-free clears the mic fault', async () => {
    await armHandsFree();
    await advance(2 * (MIC_STALL_MS + MIC_WATCHDOG_TICK_MS));
    expect(hook.current!.state).toBe('error');
    await act(async () => { await hook.current!.setHandsFree(false); });
    expect(hook.current!.error).toBeNull();
    expect(hook.current!.state).toBe('ready');
  });

  // ── The verdicts leave the hook ─────────────────────────────────────────
  // The popped-out chat window's orb has no engine; it reads what this hook
  // relays (VOICE_VERDICT_KEY). And QA could not confirm either verdict from
  // daemon.err — console lines only — so the hook also keeps a bounded event
  // trail WKWebView persists to disk. These check what the hook WRITES; the
  // VoiceOrb label tests check that the mirror orb renders it.
  describe('relays its verdicts for the mirror orb and for QA', () => {
    beforeEach(() => localStorage.clear());

    it('relays the wake hint while gated, then the deaf verdict once it fails open', async () => {
      const ws = await armHandsFree();
      act(() => ws.frame({ type: 'wake_status', active: true, phrase: 'Hey Henry' }));
      await flush();
      expect(readVoiceVerdict()).toEqual({ error: null, wakeHint: 'Say "Hey Henry"', spotterDeaf: false });

      await failOpenAt(ws, HFP_VOICE);
      expect(readVoiceVerdict()).toEqual({ error: null, wakeHint: null, spotterDeaf: true });
      const log = readVoiceVerdictLog();
      expect(log.map(e => e.event)).toContain('wake_gate_failed_open');
      expect(log.find(e => e.event === 'wake_gate_failed_open')!.detail).toMatch(/voiced episodes ignored/);
    });

    it('relays the watchdog sentence, logs rebuild → surfaced → cleared, and clears on unmount', async () => {
      await armHandsFree();
      await advance(2 * (MIC_STALL_MS + MIC_WATCHDOG_TICK_MS));
      expect(hook.current!.state).toBe('error');
      expect(readVoiceVerdict()?.error).toBe(MIC_FAULT_MESSAGE);

      act(() => pump());
      await advance(MIC_WATCHDOG_TICK_MS);
      expect(readVoiceVerdict()?.error).toBeNull();

      expect(readVoiceVerdictLog().map(e => e.event))
        .toEqual(['mic_rebuild', 'mic_fault_surfaced', 'mic_fault_cleared']);
      expect(readVoiceVerdictLog()[1].detail).toContain(MIC_FAULT_MESSAGE);

      // No engine, no verdict: a mirror must not read a dead window's word.
      act(() => root.render(<></>));
      expect(localStorage.getItem(VOICE_VERDICT_KEY)).toBeNull();
      // The trail survives — it is for after-the-fact checks.
      expect(readVoiceVerdictLog()).toHaveLength(3);
    });

    it('keeps the trail bounded — a flapping mic cannot grow localStorage', () => {
      for (let i = 0; i < VOICE_VERDICT_LOG_MAX + 7; i++) recordVoiceVerdictEvent('mic_rebuild', `n${i}`);
      const log = readVoiceVerdictLog();
      expect(log).toHaveLength(VOICE_VERDICT_LOG_MAX);
      expect(log[log.length - 1].detail).toBe(`n${VOICE_VERDICT_LOG_MAX + 6}`);
    });
  });
});
