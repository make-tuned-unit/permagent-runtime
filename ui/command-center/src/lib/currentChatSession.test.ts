import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('./api', async () => {
  const actual = await vi.importActual<typeof import('./api')>('./api');
  return {
    ...actual,
    api: {
      ...actual.api,
      newCurrentChatSession: vi.fn(),
      setCurrentChatSession: vi.fn(),
    },
  };
});

import { api } from './api';
import { applyLivenessFrame, _resetLivenessSync } from './livenessSync';
import { useCommandCenter } from './store';

const newCurrent = vi.mocked(api.newCurrentChatSession);

describe('suite-wide current chat session', () => {
  beforeEach(() => {
    _resetLivenessSync();
    newCurrent.mockReset();
    useCommandCenter.setState({
      chatSessionId: 'a',
      currentChatSessionId: 'a',
      pendingFollowSessionId: null,
      switchToSession: vi.fn(async () => {}),
    } as never);
  });

  it('a current_chat_session_changed frame from another surface queues a follow', () => {
    applyLivenessFrame(
      { type: 'current_chat_session_changed', payload: { session_id: 'b', name: 'n', origin_client: 'ios' } },
      0,
    );
    const s = useCommandCenter.getState();
    expect(s.currentChatSessionId).toBe('b');
    expect(s.pendingFollowSessionId).toBe('b');
  });

  it('the echo of our own change queues nothing (we already show it)', () => {
    applyLivenessFrame(
      { type: 'current_chat_session_changed', payload: { session_id: 'a', name: 'n', origin_client: 'desktop' } },
      0,
    );
    expect(useCommandCenter.getState().pendingFollowSessionId).toBeNull();
  });

  it('dock "new chat" creates through the current-session route and shows it', async () => {
    newCurrent.mockResolvedValue({ session_id: 'fresh', name: 'New Chat', created: true });
    const id = await useCommandCenter.getState().startNewCurrentChat();
    expect(id).toBe('fresh');
    expect(newCurrent).toHaveBeenCalledWith('desktop');
    expect(useCommandCenter.getState().currentChatSessionId).toBe('fresh');
    expect(useCommandCenter.getState().switchToSession).toHaveBeenCalledWith('fresh');
  });

  it('a failed new chat reports null and changes nothing', async () => {
    newCurrent.mockRejectedValue(new Error('down'));
    expect(await useCommandCenter.getState().startNewCurrentChat()).toBeNull();
    expect(useCommandCenter.getState().currentChatSessionId).toBe('a');
  });
});
