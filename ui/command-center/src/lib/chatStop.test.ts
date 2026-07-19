/**
 * Chat Stop/interrupt button — store-logic tests.
 *
 * Covers the wiring that lets the composer's Stop button cancel an in-flight
 * turn, and the ActiveRequests truth signal in BOTH directions (C1/C4): a
 * non-empty list adopts the request_id AND streaming state (a mid-turn attach
 * gets an honest composer + Stop button); an EMPTY list reconciles to idle —
 * it is the daemon's only "nothing is running" signal after a turn died
 * without a terminal frame (e.g. daemon restart). stopStreaming acts on the
 * cancel endpoint's honest {cancelled} answer: false means no terminal frame
 * is coming, so streaming state is reconciled immediately. Terminal events
 * (Finish/Error) still clear the tracked id so the UI returns to idle.
 * `./api` is mocked so no network is touched; the store's own reload helpers are
 * stubbed via setState so the Finish side-effect cascade stays out of the way.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';
import type { SSEEvent, TokenState } from './api';

const { cancelReply } = vi.hoisted(() => ({
  cancelReply: vi.fn((_sessionId: string, _requestId: string) =>
    Promise.resolve({ cancelled: true })),
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
  cancelReply.mockResolvedValue({ cancelled: true });
  useCommandCenter.setState({
    isStreaming: false,
    _activeRequestId: null,
    _streamingMessageId: null,
    chatSessionId: null,
    chatMessages: [],
  });
});

describe('active request tracking', () => {
  it('adopts the request_id AND streaming state from an ActiveRequests frame (mid-turn attach)', () => {
    // A window connecting mid-turn (reload, detached dock) must show an honest
    // composer: Stop button present, send disabled — not an idle input whose
    // send would 400 against the busy session (C4).
    const frame: SSEEvent = { type: 'ActiveRequests', request_ids: ['req-abc'] };
    useCommandCenter.getState().handleSessionEvent(frame);
    const s = useCommandCenter.getState();
    expect(s._activeRequestId).toBe('req-abc');
    expect(s.isStreaming).toBe(true);
  });

  it('keeps the streaming placeholder across a mid-turn reconnect adopt', () => {
    // Same-window reconnect: the placeholder bubble keeps receiving deltas.
    useCommandCenter.setState({ isStreaming: true, _streamingMessageId: 'msg-stream', _activeRequestId: 'req-old' });
    const frame: SSEEvent = { type: 'ActiveRequests', request_ids: ['req-old'] };
    useCommandCenter.getState().handleSessionEvent(frame);
    expect(useCommandCenter.getState()._streamingMessageId).toBe('msg-stream');
  });

  it('reconciles to idle on an EMPTY ActiveRequests list (turn died without a terminal frame)', () => {
    // Daemon restarted mid-turn: fresh bus, empty replay, no Finish/Error is
    // ever coming. The empty list is the only "nothing is running" signal —
    // acting on it is what un-wedges the composer (C1).
    useCommandCenter.setState({ isStreaming: true, _activeRequestId: 'req-dead', _streamingMessageId: 'msg-x' });
    const frame: SSEEvent = { type: 'ActiveRequests', request_ids: [] };
    useCommandCenter.getState().handleSessionEvent(frame);
    const s = useCommandCenter.getState();
    expect(s.isStreaming).toBe(false);
    expect(s._activeRequestId).toBeNull();
    expect(s._streamingMessageId).toBeNull();
  });

  it('does NOT reconcile on an empty list while the reply POST is still in flight', () => {
    // isStreaming set optimistically, request_id not yet returned: the server
    // may simply not have registered the request when the frame was emitted.
    // A racing reconnect must not kill a turn that is being born.
    useCommandCenter.setState({ isStreaming: true, _activeRequestId: null, _streamingMessageId: 'msg-y' });
    const frame: SSEEvent = { type: 'ActiveRequests', request_ids: [] };
    useCommandCenter.getState().handleSessionEvent(frame);
    const s = useCommandCenter.getState();
    expect(s.isStreaming).toBe(true);
    expect(s._streamingMessageId).toBe('msg-y');
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
    // A real cancel settles via the daemon's terminal Finish — streaming stays
    // on until it lands (resetting early would race the request slot).
    expect(useCommandCenter.getState().isStreaming).toBe(true);
  });

  it('reconciles to idle when the daemon says nothing was cancelled (stale id)', async () => {
    // {cancelled:false}: the turn already ended or the daemon restarted — no
    // terminal frame is coming, so waiting would spin the Stop button forever.
    cancelReply.mockResolvedValueOnce({ cancelled: false });
    useCommandCenter.setState({ isStreaming: true, chatSessionId: 'sess-1', _activeRequestId: 'req-gone', _streamingMessageId: 'msg-z' });
    const issued = await useCommandCenter.getState().stopStreaming();
    expect(issued).toBe(false);
    const s = useCommandCenter.getState();
    expect(s.isStreaming).toBe(false);
    expect(s._activeRequestId).toBeNull();
    expect(s._streamingMessageId).toBeNull();
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
