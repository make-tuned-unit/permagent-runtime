// @vitest-environment jsdom
/**
 * The open chat pane follows turns made on another device (Watch/iPhone relay
 * -> POST /reply on the same session id), and re-opening a session from
 * History re-reads it from the hub instead of showing a stale copy.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

const store: Record<string, string> = {};
vi.stubGlobal('localStorage', {
  getItem: (k: string) => (k in store ? store[k] : null),
  setItem: (k: string, v: string) => { store[k] = v; },
  removeItem: (k: string) => { delete store[k]; },
  clear: () => {},
});

const { getSession, sessionEventsUrl } = vi.hoisted(() => ({
  getSession: vi.fn(),
  sessionEventsUrl: vi.fn(() => Promise.resolve('http://x/events')),
}));

vi.mock('./api', () => ({
  api: { getSession, sessionEventsUrl, getSessions: vi.fn(() => Promise.resolve([])) },
  apiFetch: vi.fn(),
  extractText: (m: { content: { type: string; text?: string }[] }) =>
    m.content.filter(c => c.type === 'text').map(c => c.text).join(''),
  extractSystemNotice: vi.fn(() => null),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

import { useCommandCenter } from './store';
import { syncOpenSessionOnce, LIVE_SESSION_POLL_MS } from './liveSessionSync';

const msg = (id: string, role: 'user' | 'assistant', text: string) =>
  ({ id, role, created: 1, content: [{ type: 'text', text }], metadata: {} });
const session = (conversation: unknown[]) => ({
  id: 's1', provider_name: 'p', model_config: { model_name: 'm' }, conversation,
});

class FakeES { onopen = null; onmessage = null; onerror = null; close() {} }

beforeEach(() => {
  getSession.mockReset();
  vi.stubGlobal('EventSource', FakeES);
  useCommandCenter.setState({
    chatSessionId: null, chatMessages: [], isStreaming: false, chatHistoryLoaded: false,
  });
});
afterEach(() => { vi.unstubAllGlobals(); });

describe('live session sync', () => {
  it('shows a turn made by another client in the open pane without user action', async () => {
    getSession.mockResolvedValueOnce(session([msg('a', 'user', 'hi'), msg('b', 'assistant', 'hello')]));
    await useCommandCenter.getState().switchToSession('s1');
    expect(useCommandCenter.getState().chatMessages.map(m => m.content)).toEqual(['hi', 'hello']);

    // The Watch adds a user turn and Aria's reply on the same session id.
    getSession.mockResolvedValueOnce(session([
      msg('a', 'user', 'hi'), msg('b', 'assistant', 'hello'),
      msg('c', 'user', 'from the watch'), msg('d', 'assistant', 'reply to the watch'),
    ]));
    await syncOpenSessionOnce();
    expect(useCommandCenter.getState().chatMessages.map(m => m.content)).toEqual([
      'hi', 'hello', 'from the watch', 'reply to the watch',
    ]);
    expect(LIVE_SESSION_POLL_MS).toBeGreaterThan(0);
  });

  it('does not touch the pane when nothing changed, or while a local turn streams', async () => {
    const conv = [msg('a', 'user', 'hi')];
    getSession.mockResolvedValue(session(conv));
    await useCommandCenter.getState().switchToSession('s1');
    const before = useCommandCenter.getState().chatMessages;
    await syncOpenSessionOnce();
    expect(useCommandCenter.getState().chatMessages).toBe(before);

    useCommandCenter.setState({ isStreaming: true });
    getSession.mockClear();
    await syncOpenSessionOnce();
    expect(getSession).not.toHaveBeenCalled();
  });

  it('reopening a session reloads it from the hub (server-side truncation shows)', async () => {
    getSession.mockResolvedValueOnce(session([msg('a', 'user', 'one'), msg('b', 'assistant', 'two')]));
    await useCommandCenter.getState().switchToSession('s1');
    expect(useCommandCenter.getState().chatMessages).toHaveLength(2);

    getSession.mockResolvedValueOnce(session([msg('a', 'user', 'one')])); // truncated on the hub
    await useCommandCenter.getState().switchToSession('s1');
    expect(getSession).toHaveBeenCalledTimes(2);
    expect(useCommandCenter.getState().chatMessages.map(m => m.content)).toEqual(['one']);
  });

  it('drops a slow reply for a session that is no longer open', async () => {
    let release!: (v: unknown) => void;
    getSession.mockReturnValueOnce(new Promise(r => { release = r; }));
    const slow = useCommandCenter.getState().loadSessionMessages('old');
    useCommandCenter.setState({ chatSessionId: 'other' });
    release(session([msg('z', 'user', 'stale')]));
    await slow;
    expect(useCommandCenter.getState().chatMessages).toEqual([]);
  });
});
