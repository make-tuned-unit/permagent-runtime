/**
 * Chat Stop/interrupt button — store-logic tests.
 *
 * Covers the wiring that lets the composer's Stop button cancel an in-flight
 * turn: tracking the active request_id (adopted from an ActiveRequests frame on
 * reconnect), the `stopStreaming` action that POSTs the cancel, and the terminal
 * events (Finish/Error) that clear the tracked id so the UI returns to idle.
 * `./api` is mocked so no network is touched; the store's own reload helpers are
 * stubbed via setState so the Finish side-effect cascade stays out of the way.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';
import type { SSEEvent, TokenState } from './api';

const { cancelReply } = vi.hoisted(() => ({
  cancelReply: vi.fn((_sessionId: string, _requestId: string) => Promise.resolve(undefined)),
}));

vi.mock('./api', () => ({
  api: { cancelReply, sendReply: vi.fn() },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { useCommandCenter } from './store';

function tokenState(): TokenState {
  return {
    inputTokens: 0, outputTokens: 0, totalTokens: 0,
    accumulatedInputTokens: 0, accumulatedOutputTokens: 0, accumulatedTotalTokens: 0,
    costUsd: 0, accumulatedCostUsd: 0, cacheSavingsUsd: 0, contextPercent: null, model: '',
  };
}

beforeEach(() => {
  cancelReply.mockReset();
  cancelReply.mockResolvedValue(undefined);
  useCommandCenter.setState({
    isStreaming: false,
    _activeRequestId: null,
    _streamingMessageId: null,
    chatSessionId: null,
    chatMessages: [],
  });
});

describe('active request tracking', () => {
  it('adopts the request_id from an ActiveRequests frame (reconnect reattach)', () => {
    const frame: SSEEvent = { type: 'ActiveRequests', request_ids: ['req-abc'] };
    useCommandCenter.getState().handleSessionEvent(frame);
    expect(useCommandCenter.getState()._activeRequestId).toBe('req-abc');
  });

  it('ignores an empty ActiveRequests list', () => {
    useCommandCenter.setState({ _activeRequestId: 'keep' });
    const frame: SSEEvent = { type: 'ActiveRequests', request_ids: [] };
    useCommandCenter.getState().handleSessionEvent(frame);
    expect(useCommandCenter.getState()._activeRequestId).toBe('keep');
  });

  it('clears the active request_id and streaming flag when the turn finishes', () => {
    useCommandCenter.setState({
      isStreaming: true,
      _activeRequestId: 'req-1',
      chatSessionId: 's1',
      // Neutralize the Finish side-effect cascade (skills/proposals/transcript reload).
      loadProposals: vi.fn(),
      loadSkills: vi.fn(),
      loadSessionMessages: vi.fn(() => Promise.resolve()),
    });
    const frame: SSEEvent = { type: 'Finish', reason: 'stop', token_state: tokenState() };
    useCommandCenter.getState().handleSessionEvent(frame);
    const s = useCommandCenter.getState();
    expect(s.isStreaming).toBe(false);
    expect(s._activeRequestId).toBeNull();
  });

  it('clears the active request_id on an Error frame', () => {
    useCommandCenter.setState({ isStreaming: true, _activeRequestId: 'req-2' });
    const frame: SSEEvent = { type: 'Error', error: 'boom' };
    useCommandCenter.getState().handleSessionEvent(frame);
    const s = useCommandCenter.getState();
    expect(s.isStreaming).toBe(false);
    expect(s._activeRequestId).toBeNull();
  });
});

describe('stopStreaming', () => {
  it('POSTs cancel for the active request while streaming and reports it', async () => {
    useCommandCenter.setState({ isStreaming: true, chatSessionId: 'sess-1', _activeRequestId: 'req-9' });
    const issued = await useCommandCenter.getState().stopStreaming();
    expect(issued).toBe(true);
    expect(cancelReply).toHaveBeenCalledWith('sess-1', 'req-9');
  });

  it('is a no-op (returns false) when not streaming', async () => {
    useCommandCenter.setState({ isStreaming: false, chatSessionId: 'sess-1', _activeRequestId: 'req-9' });
    const issued = await useCommandCenter.getState().stopStreaming();
    expect(issued).toBe(false);
    expect(cancelReply).not.toHaveBeenCalled();
  });

  it('is a no-op (returns false) when the request_id has not landed yet', async () => {
    useCommandCenter.setState({ isStreaming: true, chatSessionId: 'sess-1', _activeRequestId: null });
    const issued = await useCommandCenter.getState().stopStreaming();
    expect(issued).toBe(false);
    expect(cancelReply).not.toHaveBeenCalled();
  });

  it('propagates a cancel failure so the caller can re-enable Stop', async () => {
    cancelReply.mockRejectedValueOnce(new Error('HTTP 404'));
    useCommandCenter.setState({ isStreaming: true, chatSessionId: 'sess-1', _activeRequestId: 'req-9' });
    await expect(useCommandCenter.getState().stopStreaming()).rejects.toThrow('HTTP 404');
  });
});
