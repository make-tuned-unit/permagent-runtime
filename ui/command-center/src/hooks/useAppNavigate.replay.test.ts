/**
 * Replay honesty for the /events ACTION path — wiring tests.
 *
 * The daemon replays its whole event buffer on every (re)connect, stamping
 * each re-delivered frame `"replayed": true` server-side. `shouldActOnFrame`
 * (via worldEvents' `frameReplayed`) treats that marker as authoritative and
 * keeps the per-connection epoch over the daemon's own timestamps as the
 * fallback for unmarked frames (older daemons; frames emitted while
 * disconnected), and `routeGlobalFrame` enforces "replayed frames are
 * RECORDED in the trace but never ACTED on". Without this gate a sleep/wake
 * reconnect re-delivered an hours-old app_navigate and yanked the user.
 *
 * `../lib/api` is mocked (the openItem-test pattern) so importing the store +
 * hook module touches no network. `reason` is omitted from every payload so
 * the DOM-only navigation cue never runs under the node test env.
 */

import { describe, expect, it, beforeEach, vi } from 'vitest';

vi.mock('../lib/api', () => ({
  api: {},
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { shouldActOnFrame, routeGlobalFrame } from './useAppNavigate';
import { useCommandCenter } from '../lib/store';
import { _resetTraceDedupe } from '../lib/traceEvents';

/** Connection opened at noon; the daemon's replay burst predates it. */
const EPOCH = Date.parse('2026-07-18T12:00:00Z');
const STALE_TS = '2026-07-18T09:00:00Z'; // hours-old buffered frame
const LIVE_TS = '2026-07-18T12:00:05Z'; // emitted after this connection opened

const ACTION_TYPES = ['app_navigate', 'app_action', 'app_open_item', 'project_launch', 'app_clipboard'];

let seq = 0;
function frame(type: string, payload: Record<string, unknown>, timestamp?: string): unknown {
  return { id: `evt-${seq++}`, type, timestamp, payload };
}

function handlers() {
  return { navigate: vi.fn(), launch: vi.fn(), theme: 'dark' as const };
}

beforeEach(() => {
  _resetTraceDedupe();
  useCommandCenter.setState({ events: [], goalDetail: null, chatDockOpen: false, workspaces: [] });
});

describe('shouldActOnFrame — per-connection replay gate', () => {
  it('rejects every action type when the frame predates the connection', () => {
    for (const type of ACTION_TYPES) {
      expect(shouldActOnFrame(frame(type, {}, STALE_TS), EPOCH)).toBe(false);
    }
  });

  it('passes every action type when the frame is live for this connection', () => {
    for (const type of ACTION_TYPES) {
      expect(shouldActOnFrame(frame(type, {}, LIVE_TS), EPOCH)).toBe(true);
    }
  });

  it('never acts on non-action frames, even live ones (trace-only)', () => {
    expect(shouldActOnFrame(frame('agent_state_changed', { state: 'working' }, LIVE_TS), EPOCH)).toBe(false);
    expect(shouldActOnFrame(frame('memory_recalled', { query: 'q' }, LIVE_TS), EPOCH)).toBe(false);
  });

  it('treats an untimestamped action frame as live (daemon stamps everything; absence = test frame)', () => {
    expect(shouldActOnFrame(frame('app_navigate', { tool_type: 'build' }), EPOCH)).toBe(true);
  });

  it('treats a timestamp exactly at the epoch as live (only strictly-earlier frames are replay)', () => {
    expect(shouldActOnFrame(frame('app_navigate', {}, new Date(EPOCH).toISOString()), EPOCH)).toBe(true);
  });

  it('rejects malformed frames outright', () => {
    expect(shouldActOnFrame(null, EPOCH)).toBe(false);
    expect(shouldActOnFrame('app_navigate', EPOCH)).toBe(false);
    expect(shouldActOnFrame({ payload: {} }, EPOCH)).toBe(false);
  });
});

describe('shouldActOnFrame — server-side replay marker (authoritative)', () => {
  /** A frame the daemon stamped as buffer re-delivery. */
  function marked(type: string, timestamp?: string): unknown {
    return { ...(frame(type, {}, timestamp) as object), replayed: true };
  }

  it('never acts on a server-marked frame, even one stamped after this connection opened', () => {
    // A reconnect replay of a frame emitted while we were disconnected: its
    // timestamp postdates the OLD epoch and could postdate a skewed clock's
    // new epoch too — the marker closes the #770 clock-skew caveat.
    for (const type of ACTION_TYPES) {
      expect(shouldActOnFrame(marked(type, LIVE_TS), EPOCH)).toBe(false);
    }
  });

  it('never acts on a server-marked frame without a timestamp (marker needs no clock at all)', () => {
    expect(shouldActOnFrame(marked('app_navigate'), EPOCH)).toBe(false);
  });

  it('keeps the epoch fallback for unmarked frames (older daemons)', () => {
    // These frames carry no marker — exactly what an older daemon sends — and
    // must classify exactly as before the marker existed.
    expect(shouldActOnFrame(frame('app_navigate', {}, STALE_TS), EPOCH)).toBe(false);
    expect(shouldActOnFrame(frame('app_navigate', {}, LIVE_TS), EPOCH)).toBe(true);
  });

  it('an explicit replayed:false does not overrule the epoch guard (marker is one-directional)', () => {
    const f = { ...(frame('app_navigate', {}, STALE_TS) as object), replayed: false };
    expect(shouldActOnFrame(f, EPOCH)).toBe(false);
  });
});

describe('routeGlobalFrame — replayed frames are recorded, never acted on', () => {
  it('records a replayed app_navigate in the trace WITHOUT navigating', () => {
    const h = handlers();
    routeGlobalFrame(frame('app_navigate', { tool_type: 'build' }, STALE_TS), EPOCH, h);
    expect(useCommandCenter.getState().events.map(e => e.event_type)).toEqual(['app_navigate']);
    expect(h.navigate).not.toHaveBeenCalled();
  });

  it('records a server-marked app_navigate in the trace WITHOUT navigating, even with a live timestamp', () => {
    const h = handlers();
    const f = { ...(frame('app_navigate', { tool_type: 'build' }, LIVE_TS) as object), replayed: true };
    routeGlobalFrame(f, EPOCH, h);
    expect(useCommandCenter.getState().events.map(e => e.event_type)).toEqual(['app_navigate']);
    expect(h.navigate).not.toHaveBeenCalled();
  });

  it('records AND navigates on a live app_navigate', () => {
    const h = handlers();
    routeGlobalFrame(frame('app_navigate', { tool_type: 'build' }, LIVE_TS), EPOCH, h);
    expect(useCommandCenter.getState().events).toHaveLength(1);
    expect(h.navigate).toHaveBeenCalledWith({ tool_type: 'build' });
  });

  it('does not launch a terminal for a replayed project_launch', () => {
    const h = handlers();
    routeGlobalFrame(frame('project_launch', { root_path: '/tmp/p' }, STALE_TS), EPOCH, h);
    expect(h.launch).not.toHaveBeenCalled();
    routeGlobalFrame(frame('project_launch', { root_path: '/tmp/p' }, LIVE_TS), EPOCH, h);
    expect(h.launch).toHaveBeenCalledWith({ root_path: '/tmp/p' });
  });

  it('does not open an item for a replayed app_open_item, but does for a live one', () => {
    const h = handlers();
    const payload = { kind: 'goal', project_id: 'p1', card_id: 'c1' };
    routeGlobalFrame(frame('app_open_item', payload, STALE_TS), EPOCH, h);
    expect(useCommandCenter.getState().goalDetail).toBeNull();
    routeGlobalFrame(frame('app_open_item', payload, LIVE_TS), EPOCH, h);
    expect(useCommandCenter.getState().goalDetail).toEqual({ projectId: 'p1', cardId: 'c1' });
  });

  it('does not act on a surface for a replayed app_action, but does for a live one', () => {
    const h = handlers();
    const payload = { surface: 'chat', action: 'open' };
    routeGlobalFrame(frame('app_action', payload, STALE_TS), EPOCH, h);
    expect(useCommandCenter.getState().chatDockOpen).toBe(false);
    routeGlobalFrame(frame('app_action', payload, LIVE_TS), EPOCH, h);
    expect(useCommandCenter.getState().chatDockOpen).toBe(true);
  });

  it('reconnect replay of an already-seen live frame: neither re-recorded nor re-acted', () => {
    const h = handlers();
    // Connection 1: the frame arrives live — acted on and recorded.
    const evt = frame('app_navigate', { tool_type: 'build' }, LIVE_TS);
    routeGlobalFrame(evt, EPOCH, h);
    expect(h.navigate).toHaveBeenCalledTimes(1);
    expect(useCommandCenter.getState().events).toHaveLength(1);
    // Connection 2 (post-sleep reconnect): the daemon replays its buffer and
    // re-delivers the SAME envelope. The new connection's epoch postdates the
    // frame, so it must not re-navigate — and the id dedupe drops the record.
    const reconnectEpoch = Date.parse('2026-07-18T15:00:00Z');
    routeGlobalFrame(evt, reconnectEpoch, h);
    expect(h.navigate).toHaveBeenCalledTimes(1);
    expect(useCommandCenter.getState().events).toHaveLength(1);
  });

  it('still records non-action frames (trace-only path unchanged)', () => {
    const h = handlers();
    routeGlobalFrame(frame('agent_state_changed', { agent_id: 'a', state: 'working' }, LIVE_TS), EPOCH, h);
    expect(useCommandCenter.getState().events.map(e => e.event_type)).toEqual(['agent_state_changed']);
    expect(h.navigate).not.toHaveBeenCalled();
    expect(h.launch).not.toHaveBeenCalled();
  });
});
