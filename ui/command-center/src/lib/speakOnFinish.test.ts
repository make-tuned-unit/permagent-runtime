/**
 * Speak-replies gating on the `Finish` frame — the "did THIS client watch the
 * turn stream in?" rule.
 *
 * Regression: popping the chat out mid-turn re-spoke the session's greeting.
 * The new window adopted `isStreaming` from the `ActiveRequests` frame the
 * daemon sends on every connect, and the null-cursor replay then delivered the
 * whole session history — so the first replayed `Finish`, whose `lastAssistant`
 * is the opening reply, passed a guard that checked `isStreaming` alone.
 *
 * `_streamingMessageId` is the discriminating half: only `sendMessage` sets it,
 * and ActiveRequests adoption deliberately leaves it alone.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';
import type { SSEEvent } from './api';

const { maybeSpeakReply } = vi.hoisted(() => ({
  maybeSpeakReply: vi.fn((_markdown: string, _voiceId?: string | null, _dedupeKey?: string) =>
    Promise.resolve()),
}));

vi.mock('./speakReplies', () => ({
  maybeSpeakReply,
  replyDedupeKey: (sessionId: string | null, content: string) => `${sessionId}:${content}`,
  markReplySpoken: vi.fn(),
}));

vi.mock('./api', () => ({
  // getSession: the Finish handler rehydrates from the daemon afterwards.
  api: { cancelReply: vi.fn(), sendReply: vi.fn(), getSession: vi.fn(() => Promise.resolve(null)) },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

import { useCommandCenter } from './store';

const GREETING = 'Lets begin.';

beforeEach(() => {
  maybeSpeakReply.mockClear();
  useCommandCenter.setState({
    isStreaming: false,
    _activeRequestId: null,
    _streamingMessageId: null,
    chatSessionId: 'session-1',
    chatMessages: [
      {
        id: 'msg-greeting',
        role: 'assistant' as const,
        content: GREETING,
        timestamp: new Date().toISOString(),
      },
    ],
    // Keep the Finish side-effect cascade out of the way.
    loadProposals: vi.fn(),
    loadSkills: vi.fn(),
  });
});

function finish(): SSEEvent {
  return { type: 'Finish' } as unknown as SSEEvent;
}

function activeRequests(ids: string[]): SSEEvent {
  return { type: 'ActiveRequests', request_ids: ids } as unknown as SSEEvent;
}

describe('speak-replies gating on Finish', () => {
  it('stays silent on a replayed Finish after a mid-turn attach adopted isStreaming', () => {
    // Popped-out chat window: connects, adopts the live turn from
    // ActiveRequests, then receives the replayed history.
    useCommandCenter.getState().handleSessionEvent(activeRequests(['req-live']));
    expect(useCommandCenter.getState().isStreaming).toBe(true);
    expect(useCommandCenter.getState()._streamingMessageId).toBeNull();

    useCommandCenter.getState().handleSessionEvent(finish());

    expect(maybeSpeakReply).not.toHaveBeenCalled();
  });

  it('stays silent on a replayed Finish with no streaming state at all', () => {
    useCommandCenter.getState().handleSessionEvent(finish());
    expect(maybeSpeakReply).not.toHaveBeenCalled();
  });

  it('speaks the turn the window itself sent and watched stream in', () => {
    // What sendMessage leaves behind: its own placeholder id is tracked.
    useCommandCenter.setState({ isStreaming: true, _streamingMessageId: 'msg-mine' });

    useCommandCenter.getState().handleSessionEvent(finish());

    expect(maybeSpeakReply).toHaveBeenCalledTimes(1);
    expect(maybeSpeakReply.mock.calls[0][0]).toBe(GREETING);
  });
});
