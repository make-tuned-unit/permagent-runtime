/**
 * "Discuss with {agent}" reads as "open a discussion". It is also a session
 * swap: whatever conversation was on screen — possibly mid-stream — is
 * replaced by a fresh one. Nothing is destroyed, the old session is in
 * Sessions, and nothing on screen said either of those things. From the user's
 * side, a chat vanished because they pressed a button about something else.
 *
 * And when the new session could not be created at all, the whole action was a
 * `console.error` and a button that did nothing.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';

const { createSession } = vi.hoisted(() => ({ createSession: vi.fn() }));

vi.mock('./api', () => ({
  api: {
    createSession,
    // The swap that follows the notice reaches the network. Stubbed to inert
    // so the assertions are about the notice, which is set before any of it.
    sessionEventsUrl: vi.fn(async () => 'http://localhost:1234/events'),
    sendMessage: vi.fn(async () => ({})),
    sendReply: vi.fn(async () => ({})),
  },
  apiFetch: vi.fn(async () => ({})),
  getApiBaseUrl: () => 'http://localhost:1234',
  loadDaemonToken: vi.fn(),
}));

// jsdom is not loaded for this file (the store is plain state), and the swap
// opens an SSE channel on its way past.
class InertEventSource {
  close() {}
  addEventListener() {}
  removeEventListener() {}
}
(globalThis as Record<string, unknown>).EventSource = InertEventSource;

import { useCommandCenter } from './store';

beforeEach(() => {
  createSession.mockReset();
  useCommandCenter.setState({ discussNotice: null, chatSessionId: null });
});

describe('discussDecision', () => {
  it('says what it replaced, and where it went', async () => {
    createSession.mockResolvedValue({ id: 'new-session' });
    useCommandCenter.setState({ chatSessionId: 'the-chat-i-was-in' });
    // The rest of the swap (connect, send the seed turn) reaches the network;
    // the notice is set before any of it, which is the point — it is the part
    // that must survive a slow or failed send.
    await useCommandCenter.getState().discussDecision('d1', 'Ship the thing?').catch(() => {});
    const notice = useCommandCenter.getState().discussNotice;
    expect(notice?.tone).toBe('info');
    expect(notice?.text).toContain('Sessions');
  });

  it('does not announce a swap when there was nothing to swap', async () => {
    createSession.mockResolvedValue({ id: 'new-session' });
    useCommandCenter.setState({ chatSessionId: null });
    await useCommandCenter.getState().discussDecision('d1', 'Ship the thing?').catch(() => {});
    // A first conversation replaced nothing; saying so would be noise.
    expect(useCommandCenter.getState().discussNotice).toBeNull();
  });

  it('says so when no conversation could be started', async () => {
    createSession.mockRejectedValue(new Error('daemon unreachable'));
    const ok = await useCommandCenter.getState().discussDecision('d1', 'Ship the thing?');
    expect(ok).toBe(false);
    const notice = useCommandCenter.getState().discussNotice;
    expect(notice?.tone).toBe('error');
    expect(notice?.text).toContain('daemon unreachable');
  });

  it('is dismissible', async () => {
    createSession.mockRejectedValue(new Error('nope'));
    await useCommandCenter.getState().discussDecision('d1', 'x');
    useCommandCenter.getState().clearDiscussNotice();
    expect(useCommandCenter.getState().discussNotice).toBeNull();
  });
});
