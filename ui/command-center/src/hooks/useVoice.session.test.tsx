/**
 * @vitest-environment jsdom
 *
 * useVoice — session rebinding (bug-sweep wave 2, HIGH).
 *
 * The voice socket bakes `session_id` into its URL at connect time. The old
 * session-change effect reconnected only in processing/playing/error — in
 * 'ready' (mic active, idle between turns) the old socket survived a
 * conversation switch, so the next push-to-talk landed the utterance in the
 * PREVIOUS session. These tests lock down:
 *   - 'ready'-state rebind (the exact case that was broken): old socket
 *     closed, new socket URL carries the new session id;
 *   - mid-recording switch: the in-flight utterance is DROPPED (no 'stop' is
 *     ever sent to the old session) with a console-visible notice;
 *   - mid-connect switch: the stale connect is superseded (epoch guard) — no
 *     socket bound to the old session survives;
 *   - idle no-op: with voice off, a session switch opens nothing.
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

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
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
  // The hook never uses onopen — it waits for the server 'ready' JSON frame —
  // so the mock is born OPEN and readiness is simulated via serverReady().
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

  /** Simulate the daemon's `{"type":"ready"}` frame. */
  serverReady() {
    this.onmessage?.({ data: JSON.stringify({ type: 'ready' }) });
  }

  /** JSON frames sent by the client (start/stop control messages). */
  jsonSent(): Array<{ type: string }> {
    return this.sent
      .filter((d): d is string => typeof d === 'string')
      .map(d => JSON.parse(d) as { type: string });
  }
}

// ── Audio mocks (mic + AudioContext) ────────────────────────────────────────

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

/** Flush pending microtasks + effects (the connect path awaits the token). */
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

/** Activate voice and drive the socket to 'ready'. Returns that socket. */
async function activateTo(sessionId: string): Promise<MockWebSocket> {
  render(sessionId);
  act(() => { void hook.current!.activate(); });
  await flush();
  expect(MockWebSocket.instances.length).toBe(1);
  const ws = MockWebSocket.instances[0];
  act(() => ws.serverReady());
  await flush();
  expect(hook.current!.state).toBe('ready');
  return ws;
}

beforeEach(() => {
  MockWebSocket.instances = [];
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

describe('useVoice — session rebinding', () => {
  it('sends exactly one wake_start while hands-free activation awaits acknowledgement', async () => {
    render('session-A');
    act(() => { void hook.current!.setHandsFree(true); });
    await flush();

    const ws = MockWebSocket.instances[0];
    act(() => ws.serverReady());
    await flush();

    expect(ws.jsonSent().filter(m => m.type === 'wake_start')).toHaveLength(1);
    expect(hook.current!.handsFree).toBe(true);
  });

  it("rebinds the socket in 'ready' state: old socket closed, new URL carries the new session id (the broken case)", async () => {
    const ws1 = await activateTo('session-A');
    expect(ws1.url).toContain('session_id=session-A');

    render('session-B');
    await flush();

    // Old socket must be gone — it would have routed the next utterance to A.
    expect(ws1.closed).toBe(true);
    expect(MockWebSocket.instances.length).toBe(2);
    const ws2 = MockWebSocket.instances[1];
    expect(ws2.url).toContain('session_id=session-B');
    expect(ws2.url).not.toContain('session-A');

    // And the reconnect completes back to a usable 'ready'.
    act(() => ws2.serverReady());
    await flush();
    expect(hook.current!.state).toBe('ready');
  });

  it('drops an in-flight recording on session switch — never sends stop to the old session', async () => {
    const ws1 = await activateTo('session-A');

    act(() => hook.current!.startRecording());
    expect(hook.current!.state).toBe('recording');
    expect(ws1.jsonSent().some(m => m.type === 'start')).toBe(true);

    render('session-B');
    await flush();

    // The utterance was dropped, not committed: no 'stop' ever reached ws1,
    // so the daemon never runs the half-captured turn against session A.
    expect(ws1.jsonSent().some(m => m.type === 'stop')).toBe(false);
    expect(ws1.closed).toBe(true);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('session changed mid-recording'));

    const ws2 = MockWebSocket.instances[1];
    expect(ws2.url).toContain('session_id=session-B');
  });

  it('supersedes a connect still in flight to the old session (epoch guard)', async () => {
    // Hold every connect at the token await so the switch happens mid-connect.
    const gates: Array<(t: string) => void> = [];
    loadDaemonToken.mockImplementation(
      () => new Promise<string>(res => { gates.push(res); }),
    );

    render('session-A');
    act(() => { void hook.current!.activate(); });
    await flush();
    // Suspended before socket creation — nothing constructed yet.
    expect(MockWebSocket.instances.length).toBe(0);
    expect(hook.current!.state).toBe('connecting');

    render('session-B');
    await flush();
    // Release both held connects; the stale session-A one must abort.
    act(() => gates.forEach(g => g('tok-test')));
    await flush();

    const urls = MockWebSocket.instances.map(w => w.url);
    expect(urls.some(u => u.includes('session_id=session-A'))).toBe(false);
    const open = MockWebSocket.instances.filter(w => !w.closed);
    expect(open.length).toBe(1);
    expect(open[0].url).toContain('session_id=session-B');

    act(() => open[0].serverReady());
    await flush();
    expect(hook.current!.state).toBe('ready');
  });

  it('is a no-op while voice is off — a session switch opens no socket', async () => {
    render('session-A');
    await flush();
    render('session-B');
    await flush();
    expect(MockWebSocket.instances.length).toBe(0);
    expect(hook.current!.state).toBe('idle');
  });
});
