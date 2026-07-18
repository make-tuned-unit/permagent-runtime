/**
 * Streaming-lifecycle store logic — session-load honesty (C8) and the SSE
 * resume cursor (P1).
 *
 * C8: only a 404 (session truly gone) may disown the stored session id; a
 * transient failure keeps the id and surfaces inline via sessionLoadError
 * (MessageList renders it with a Retry — the #568 lesson), and switchToSession
 * never opens an SSE channel to a session the store just disowned.
 *
 * P1: connectSession sends the recorded `_lastEventId` back to the daemon as
 * `?last_event_id=` so a mid-turn reconnect resumes the replay instead of
 * repeating the whole buffer (duplicate deltas/error bubbles). The cursor is
 * per-session and must reset when the connection targets a different session.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { getSession, sessionEventsUrl } = vi.hoisted(() => ({
  getSession: vi.fn(),
  sessionEventsUrl: vi.fn((sessionId: string, lastEventId?: string | null) =>
    `http://test/sessions/${sessionId}/events${lastEventId ? `?last_event_id=${lastEventId}` : ''}`),
}));

vi.mock('./api', () => ({
  api: { getSession, sessionEventsUrl, cancelReply: vi.fn(), sendReply: vi.fn() },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { useCommandCenter } from './store';

// The switchToSession tests stub connectSession via setState, which sticks for
// the store's lifetime — capture the real action so beforeEach can restore it
// for the P1 tests that drive the genuine implementation.
const realConnectSession = useCommandCenter.getState().connectSession;

/** Minimal EventSource double: records constructor URLs, supports close(). */
class FakeEventSource {
  static instances: FakeEventSource[] = [];
  url: string;
  onopen: (() => void) | null = null;
  onmessage: ((ev: { lastEventId: string; data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor(url: string) {
    this.url = url;
    FakeEventSource.instances.push(this);
  }
  close() { this.closed = true; }
}

function err404(): Error & { status?: number } {
  const e = new Error('Session sess-gone not found') as Error & { status?: number };
  e.status = 404;
  return e;
}

function errTransient(): Error & { status?: number } {
  const e = new Error('fetch failed') as Error & { status?: number };
  return e;
}

beforeEach(() => {
  vi.stubGlobal('EventSource', FakeEventSource);
  FakeEventSource.instances = [];
  getSession.mockReset();
  sessionEventsUrl.mockClear();
  useCommandCenter.setState({
    chatSessionId: null,
    chatMessages: [],
    sessionLoadError: null,
    isStreaming: false,
    _streamingMessageId: null,
    _activeRequestId: null,
    _eventSource: null,
    _reconnectTimer: null,
    _lastEventId: null,
    _lastEventSessionId: null,
    connectSession: realConnectSession,
  });
});

afterEach(() => {
  useCommandCenter.getState().disconnectSession();
  vi.unstubAllGlobals();
});

describe('loadSessionMessages failure honesty (C8)', () => {
  it('disowns the session id ONLY on a 404 (session truly gone)', async () => {
    getSession.mockRejectedValueOnce(err404());
    useCommandCenter.setState({ chatSessionId: 'sess-gone' });
    await useCommandCenter.getState().loadSessionMessages('sess-gone');
    const s = useCommandCenter.getState();
    expect(s.chatSessionId).toBeNull();
    expect(s.chatMessages).toEqual([]);
    expect(s.sessionLoadError).toBeNull();
  });

  it('keeps the session id and surfaces an inline error on a transient failure', async () => {
    getSession.mockRejectedValueOnce(errTransient());
    useCommandCenter.setState({ chatSessionId: 'sess-1' });
    await useCommandCenter.getState().loadSessionMessages('sess-1');
    const s = useCommandCenter.getState();
    expect(s.chatSessionId).toBe('sess-1'); // NOT disowned
    expect(s.sessionLoadError).toBe('fetch failed');
  });

  it('clears the inline error on a successful (re)load — the Retry path', async () => {
    useCommandCenter.setState({ chatSessionId: 'sess-1', sessionLoadError: 'fetch failed' });
    getSession.mockResolvedValueOnce({
      id: 'sess-1',
      conversation: [{
        id: 'm1', role: 'user', created: 1,
        content: [{ type: 'text', text: 'hi' }],
        metadata: { userVisible: true, agentVisible: true },
      }],
    });
    await useCommandCenter.getState().loadSessionMessages('sess-1');
    const s = useCommandCenter.getState();
    expect(s.sessionLoadError).toBeNull();
    expect(s.chatMessages).toHaveLength(1);
  });
});

describe('switchToSession (C8)', () => {
  it('still connects to the target session after a transient history failure', async () => {
    getSession.mockRejectedValueOnce(errTransient());
    const connectSpy = vi.fn();
    useCommandCenter.setState({ connectSession: connectSpy });
    await useCommandCenter.getState().switchToSession('sess-2');
    const s = useCommandCenter.getState();
    expect(s.chatSessionId).toBe('sess-2'); // target kept — retry can work
    expect(s.sessionLoadError).toBe('fetch failed');
    expect(connectSpy).toHaveBeenCalledWith('sess-2');
  });

  it('does NOT open SSE to a session the 404 just disowned', async () => {
    getSession.mockRejectedValueOnce(err404());
    const connectSpy = vi.fn();
    useCommandCenter.setState({ connectSession: connectSpy });
    await useCommandCenter.getState().switchToSession('sess-gone');
    expect(useCommandCenter.getState().chatSessionId).toBeNull();
    expect(connectSpy).not.toHaveBeenCalled();
  });
});

describe('SSE resume cursor (P1)', () => {
  // connectSession is async since the auth plane (it awaits the daemon token
  // before constructing the EventSource), so every test awaits it.
  it('reconnects to the SAME session with ?last_event_id from the recorded cursor', async () => {
    useCommandCenter.setState({ _lastEventId: '42', _lastEventSessionId: 'sess-1' });
    await useCommandCenter.getState().connectSession('sess-1');
    expect(sessionEventsUrl).toHaveBeenCalledWith('sess-1', '42');
    expect(FakeEventSource.instances[0].url).toContain('last_event_id=42');
  });

  it('resets the cursor when connecting to a DIFFERENT session (seqs are per-session)', async () => {
    useCommandCenter.setState({ _lastEventId: '42', _lastEventSessionId: 'sess-1' });
    await useCommandCenter.getState().connectSession('sess-2');
    expect(sessionEventsUrl).toHaveBeenCalledWith('sess-2', null);
    expect(FakeEventSource.instances[0].url).not.toContain('last_event_id');
    const s = useCommandCenter.getState();
    expect(s._lastEventId).toBeNull();
    expect(s._lastEventSessionId).toBe('sess-2');
  });

  it('first connect (no cursor) requests the full replay', async () => {
    await useCommandCenter.getState().connectSession('sess-1');
    expect(sessionEventsUrl).toHaveBeenCalledWith('sess-1', null);
  });

  it('records the cursor from each frame\'s SSE id (what the resume sends back)', async () => {
    await useCommandCenter.getState().connectSession('sess-1');
    const es = FakeEventSource.instances[0];
    es.onmessage?.({ lastEventId: '7', data: JSON.stringify({ type: 'Ping' }) });
    expect(useCommandCenter.getState()._lastEventId).toBe('7');
  });

  it('a superseding disconnect during the token await never opens a stale stream', async () => {
    const p = useCommandCenter.getState().connectSession('sess-1');
    useCommandCenter.getState().disconnectSession(); // bumps the connect epoch
    await p;
    expect(FakeEventSource.instances).toHaveLength(0);
  });
});
