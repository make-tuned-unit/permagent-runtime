// @vitest-environment jsdom
/**
 * deleteSession data-safety — store tests.
 *
 * api.deleteSession used to be a raw fetch that RESOLVED on a 500; the store
 * then unconditionally cleared the open conversation — blanking a chat for a
 * session the daemon never deleted. api.deleteSession now throws on non-2xx and
 * the store clears chat state ONLY after a confirmed delete, propagating the
 * error so SessionsList can surface it without losing the user's open chat.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';

// jsdom here doesn't provide localStorage; stub a minimal in-memory one so the
// persisted-session-id clearing is exercised (the store guards it in try/catch,
// but we want to assert it actually clears on a confirmed delete).
let storage: Record<string, string> = {};
vi.stubGlobal('localStorage', {
  getItem: (k: string) => (k in storage ? storage[k] : null),
  setItem: (k: string, v: string) => {
    storage[k] = v;
  },
  removeItem: (k: string) => {
    delete storage[k];
  },
  clear: () => {
    storage = {};
  },
});

const { deleteSession, getSessions } = vi.hoisted(() => ({
  deleteSession: vi.fn(),
  getSessions: vi.fn(() => Promise.resolve([])),
}));

vi.mock('./api', () => ({
  api: { deleteSession, getSessions },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { useCommandCenter } from './store';

beforeEach(() => {
  deleteSession.mockReset();
  getSessions.mockReset();
  getSessions.mockResolvedValue([]);
  storage = {};
  useCommandCenter.setState({
    chatSessionId: 'open-1',
    chatMessages: [{ role: 'user', content: 'hi' } as never],
  });
  localStorage.setItem('permagent-chat-session-id', 'open-1');
});

describe('deleteSession data-safety', () => {
  it('does NOT blank the open chat when the delete fails, and propagates the error', async () => {
    deleteSession.mockRejectedValue(new Error('500'));
    await expect(useCommandCenter.getState().deleteSession('open-1')).rejects.toThrow();
    const s = useCommandCenter.getState();
    expect(s.chatSessionId).toBe('open-1');
    expect(s.chatMessages.length).toBe(1);
    expect(localStorage.getItem('permagent-chat-session-id')).toBe('open-1');
  });

  it('clears the open chat only after a confirmed delete of the OPEN session', async () => {
    deleteSession.mockResolvedValue(undefined);
    await useCommandCenter.getState().deleteSession('open-1');
    const s = useCommandCenter.getState();
    expect(s.chatSessionId).toBeNull();
    expect(s.chatMessages).toEqual([]);
    expect(localStorage.getItem('permagent-chat-session-id')).toBeNull();
    expect(getSessions).toHaveBeenCalled();
  });

  it('deleting a DIFFERENT session leaves the open chat intact', async () => {
    deleteSession.mockResolvedValue(undefined);
    await useCommandCenter.getState().deleteSession('other-9');
    const s = useCommandCenter.getState();
    expect(s.chatSessionId).toBe('open-1');
    expect(s.chatMessages.length).toBe(1);
    expect(localStorage.getItem('permagent-chat-session-id')).toBe('open-1');
  });
});
