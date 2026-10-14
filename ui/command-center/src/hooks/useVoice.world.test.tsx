/**
 * @vitest-environment jsdom
 *
 * useVoice — the World voice client (lane L4): surface in hello, per-turn
 * trigger/surface, playback acknowledgements, in-band interrupt.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { loadDaemonToken } = vi.hoisted(() => ({
  loadDaemonToken: vi.fn(() => Promise.resolve('tok-test')),
}));
vi.mock('../lib/api', () => ({ getApiBaseUrl: () => 'http://127.0.0.1:3001', loadDaemonToken }));

import { useVoice } from './useVoice';
import { useCommandCenter } from '../lib/store';
import { activeSegment, clearPlayback } from '../lib/voicePlaybackClock';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class MockWebSocket {
  static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
  static instances: MockWebSocket[] = [];
  url: string; binaryType = 'blob'; readyState = MockWebSocket.OPEN; closed = false; sent: unknown[] = [];
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((ev: { code: number; reason: string; wasClean: boolean }) => void) | null = null;
  constructor(url: string) { this.url = url; MockWebSocket.instances.push(this); }
  send(d: unknown) { this.sent.push(d); }
  close() { this.closed = true; this.readyState = MockWebSocket.CLOSED; }
  serverSend(obj: unknown) { this.onmessage?.({ data: JSON.stringify(obj) }); }
  serverAudio(n = 2400) { this.onmessage?.({ data: new Float32Array(n).fill(0.1).buffer }); }
  json(): Array<Record<string, any> & { type: string }> {
    return this.sent.filter((d): d is string => typeof d === 'string').map(d => JSON.parse(d));
  }
}

const sources: Array<{ onended: (() => void) | null }> = [];
class MockAudioContext {
  state = 'running'; destination = {}; baseLatency = 0; outputLatency = 0;
  createMediaStreamSource() { return { connect: vi.fn() }; }
  createScriptProcessor() { return { connect: vi.fn(), disconnect: vi.fn(), onaudioprocess: null }; }
  createAnalyser() { return { connect: vi.fn(), fftSize: 0, smoothingTimeConstant: 0 }; }
  createBuffer(_c: number, len: number) { return { getChannelData: () => new Float32Array(len), length: len }; }
  createBufferSource() {
    const s = { buffer: null, connect: vi.fn(), start: vi.fn(), onended: null as (() => void) | null };
    sources.push(s);
    return s;
  }
  close() { this.state = 'closed'; return Promise.resolve(); }
}
const micStream = { getTracks: () => [{ stop: vi.fn() }] };

type VoiceApi = ReturnType<typeof useVoice>;
const hook: { current: VoiceApi | null } = { current: null };
function HostWithEvents({ onEvent }: { onEvent: (e: unknown) => void }) { hook.current = useVoice({ sessionId: 's1', onEvent }); return null; }
function Host() { hook.current = useVoice({ sessionId: 's1' }); return null; }

let container: HTMLDivElement;
let root: Root;
const flush = async () => { await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); }); };

async function activate(): Promise<MockWebSocket> {
  act(() => root.render(<Host />));
  act(() => { void hook.current!.activate(); });
  await flush();
  const ws = MockWebSocket.instances[MockWebSocket.instances.length - 1];
  act(() => ws.serverSend({ type: 'ready' }));
  await flush();
  return ws;
}

const segMsg = (id: number, text = 'hello there') => ({
  type: 'audio_segment', segment_id: id, text, sample_rate: 24000, duration_ms: 100,
  timing_source: 'estimated_proportional',
  word_timings: [
    { word: 'hello', start_ms: 0, end_ms: 50, start_utf16: 0, end_utf16: 5 },
    { word: 'there', start_ms: 50, end_ms: 100, start_utf16: 6, end_utf16: 11 },
  ],
});

beforeEach(() => {
  MockWebSocket.instances = []; sources.length = 0; hook.current = null; clearPlayback();
  vi.stubGlobal('WebSocket', MockWebSocket);
  vi.stubGlobal('AudioContext', MockAudioContext);
  Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia: vi.fn(() => Promise.resolve(micStream)) } });
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  vi.spyOn(console, 'log').mockImplementation(() => {});
  vi.spyOn(console, 'info').mockImplementation(() => {});
  useCommandCenter.setState({ worldConversationActive: false });
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount()); container.remove(); vi.unstubAllGlobals(); vi.restoreAllMocks(); vi.useRealTimers();
});

describe('hello and per-turn telemetry (V-04 / V-28)', () => {
  it('declares surface "orb" in the chat pane and "world" when the World conversation is active', async () => {
    const ws = await activate();
    const hello = ws.json().find(m => m.type === 'hello')!;
    expect(hello.surface).toBe('orb');
    expect(hello.capabilities).toEqual(expect.arrayContaining(['playback_ack_v1', 'interrupt_v1', 'audio_segment_v1']));
    expect(hello).toHaveProperty('git_sha');
    expect(hello).toHaveProperty('build_number');

    // Moving into the World mid-session restates the surface.
    act(() => useCommandCenter.setState({ worldConversationActive: true }));
    const hellos = ws.json().filter(m => m.type === 'hello');
    expect(hellos).toHaveLength(2);
    expect(hellos[1].surface).toBe('world');
    act(() => hook.current!.startRecording());
    expect(ws.json().find(m => m.type === 'start')!.surface).toBe('world');
  });

  it('labels the turn trigger on start and stop', async () => {
    const ws = await activate();
    act(() => hook.current!.startRecording());
    act(() => hook.current!.stopRecording());
    expect(ws.json().find(m => m.type === 'start')!.trigger).toBe('user_tap');
    expect(ws.json().find(m => m.type === 'stop')!.trigger).toBe('user_tap');
  });
});

describe('playback acknowledgements (V-26)', () => {
  it('acks each segment start/end and the reply, in order, from the audio_segment metadata', async () => {
    const ws = await activate();
    act(() => hook.current!.startRecording());
    act(() => hook.current!.stopRecording());
    act(() => { ws.serverSend({ type: 'reply_start' }); ws.serverSend(segMsg(0)); ws.serverAudio(); });
    expect(activeSegment(performance.now())?.words.map(w => w.word)).toEqual(['hello', 'there']);
    act(() => { ws.serverSend(segMsg(1, 'again')); ws.serverAudio(); ws.serverSend({ type: 'reply_end', sample_rate: 24000 }); });
    act(() => sources[0].onended?.());
    act(() => sources[1].onended?.());
    const acks = ws.json().filter(m => ['segment_started', 'segment_ended', 'reply_playback_complete'].includes(m.type));
    expect(acks.map(a => `${a.type}:${a.segment_id ?? ''}`)).toEqual([
      'segment_started:0', 'segment_ended:0', 'segment_started:1', 'segment_ended:1', 'reply_playback_complete:',
    ]);
    expect(acks[0].turn_id).toMatch(/^web-\d+$/);
    const done = acks[4];
    expect(done.segments).toBe(2);
    expect(done.completed).toBe(true);
    expect(done.heard_ms).toBeGreaterThan(0);
    expect(done.heard_ms).toBeLessThanOrEqual(done.queued_ms + 1);
    expect(activeSegment(performance.now())).toBeNull();
  });
});

describe('in-band interrupt (V-29)', () => {
  async function speaking() {
    const ws = await activate();
    act(() => hook.current!.startRecording());
    act(() => hook.current!.stopRecording());
    act(() => { ws.serverSend({ type: 'reply_start' }); ws.serverSend(segMsg(0)); ws.serverAudio(); });
    expect(hook.current!.state).toBe('playing');
    return ws;
  }

  it('sends interrupt on the SAME socket, drops the old reply tail until `interrupted`, and does not reconnect', async () => {
    const ws = await speaking();
    act(() => hook.current!.interrupt());
    const intr = ws.json().find(m => m.type === 'interrupt')!;
    expect(intr.reason).toBe('user_tap');
    expect(intr.turn_id).toMatch(/^web-\d+$/);
    expect(ws.closed).toBe(false);
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(hook.current!.state).toBe('ready');
    expect(ws.json().some(m => m.type === 'reply_playback_complete' && m.reason === 'interrupt' && m.completed === false)).toBe(true);
    expect(activeSegment(performance.now())).toBeNull();

    // Stale tail of the cancelled reply is ignored...
    act(() => { ws.serverSend(segMsg(1)); ws.serverAudio(); ws.serverSend({ type: 'reply_end', sample_rate: 24000 }); });
    expect(hook.current!.state).toBe('ready');
    expect(sources).toHaveLength(1);
    // ...until the cut, after which a new reply plays normally.
    act(() => ws.serverSend({ type: 'interrupted', turn_id: intr.turn_id }));
    act(() => { ws.serverSend({ type: 'reply_start' }); ws.serverSend(segMsg(0)); ws.serverAudio(); });
    expect(hook.current!.state).toBe('playing');
    expect(MockWebSocket.instances).toHaveLength(1);
  });

  it('falls back to a socket reset when an older daemon never acknowledges', async () => {
    const ws = await speaking();
    vi.useFakeTimers();
    act(() => hook.current!.interrupt());
    expect(MockWebSocket.instances).toHaveLength(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(ws.closed).toBe(true);
    expect(MockWebSocket.instances).toHaveLength(2);
  });
});

describe('full reply text (D3/D5)', () => {
  it('keeps and emits the whole reply_text even when only a short part was spoken', async () => {
    const events: Array<{ type: string; text?: string }> = [];
    act(() => root.render(<HostWithEvents onEvent={e => events.push(e as never)} />));
    act(() => { void hook.current!.activate(); });
    await flush();
    const ws = MockWebSocket.instances[MockWebSocket.instances.length - 1];
    act(() => ws.serverSend({ type: 'ready' }));
    await flush();
    const full = 'Short spoken answer. Here is the long written part. And more detail on screen only.';
    act(() => {
      ws.serverSend({ type: 'reply_start' });
      ws.serverSend(segMsg(0, 'Short spoken answer.'));
      ws.serverAudio();
      ws.serverSend({ type: 'reply_text', text: full });
      ws.serverSend({ type: 'reply_end', sample_rate: 24000 });
    });
    expect(hook.current!.lastReply).toBe(full);
    expect(events.find(e => e.type === 'reply_text')?.text).toBe(full);
    expect(full.length).toBeGreaterThan('Short spoken answer.'.length);
  });
});

describe('teach path is gone (D5)', () => {
  it('ignores stray teach frames and never sends teach or teach_skip', async () => {
    const ws = await activate();
    act(() => ws.serverSend({ type: 'teach', word: 'Elspeth' }));
    act(() => ws.serverSend({ type: 'taught', word: 'Elspeth' }));
    act(() => ws.serverSend({ type: 'idle' }));
    expect(hook.current).not.toHaveProperty('teachWord');
    expect(ws.json().some(m => m.type === 'teach_skip' || m.trigger === 'teach')).toBe(false);
    act(() => hook.current!.startRecording());
    expect(ws.json().filter(m => m.type === 'start').every(m => m.trigger !== 'teach')).toBe(true);
  });
});
