/**
 * @vitest-environment jsdom
 *
 * V-10 / D1: the voice engine is bound to the dedicated voice-home session,
 * never to the chat dock's session (which may be a coding coordinator).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const useVoiceMock = vi.hoisted(() => vi.fn());
const resolveMock = vi.hoisted(() => vi.fn());

vi.mock('../../hooks/useVoice', () => ({ useVoice: useVoiceMock }));
vi.mock('../../lib/voiceHomeSession', () => ({
  resolveVoiceHomeSession: resolveMock,
  resolveCurrentChatSession: resolveMock,
  setCachedCurrentChatSession: () => undefined,
}));

import { VoiceHost } from './VoiceHost';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const lastOpts = () => useVoiceMock.mock.calls[useVoiceMock.mock.calls.length - 1][0];

function engineStub(handsFree = false) {
  return {
    state: 'idle', error: null, activate: vi.fn(), deactivate: vi.fn(),
    startRecording: vi.fn(), stopRecording: vi.fn(), interrupt: vi.fn(),
    getAnalyser: vi.fn(), getMicAnalyser: vi.fn(), handsFree,
    setHandsFree: vi.fn(async () => {}),
    wakeWord: { active: false, gated: false, phrase: null },
  };
}

describe('VoiceHost session binding', () => {
  let root: Root;
  let host: HTMLDivElement;
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    root = createRoot(host);
    useVoiceMock.mockReset();
    useVoiceMock.mockImplementation(() => engineStub());
    resolveMock.mockReset();
    useCommandCenter.setState({ chatSessionId: 'coding-session-7', voiceEngine: null, currentChatSessionId: null, pendingFollowSessionId: null, switchToSession: vi.fn(async () => {}) } as never);
  });
  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  it('passes the voice-home id, not the dock chat session, and publishes the engine only once resolved', async () => {
    let resolve!: (id: string) => void;
    resolveMock.mockReturnValue(new Promise<string>(r => { resolve = r; }));
    await act(async () => { root.render(<VoiceHost />); });

    // Unresolved: no session id handed to the engine, engine not published.
    expect(lastOpts().sessionId).toBeUndefined();
    expect(useCommandCenter.getState().voiceEngine).toBeNull();

    await act(async () => { resolve('voice-home-1'); });
    expect(lastOpts().sessionId).toBe('voice-home-1');
    expect(useCommandCenter.getState().voiceEngine).not.toBeNull();
    for (const [opts] of useVoiceMock.mock.calls) {
      expect(opts.sessionId).not.toBe('coding-session-7');
    }
  });

  it('retries when resolution fails', async () => {
    vi.useFakeTimers();
    try {
      resolveMock.mockRejectedValueOnce(new Error('down')).mockResolvedValue('voice-home-2');
      await act(async () => { root.render(<VoiceHost />); });
      expect(useCommandCenter.getState().voiceEngine).toBeNull();
      await act(async () => { await vi.advanceTimersByTimeAsync(5100); });
      expect(lastOpts().sessionId).toBe('voice-home-2');
    } finally {
      vi.useRealTimers();
    }
  });

  it('the dock adopts the current chat on first sight, and follows later changes', async () => {
    resolveMock.mockResolvedValue('current-1');
    const switchToSession = vi.fn(async () => {});
    useCommandCenter.setState({
      switchToSession, currentChatSessionId: null, pendingFollowSessionId: null, isStreaming: false,
    } as never);
    await act(async () => { root.render(<VoiceHost />); });
    expect(switchToSession).toHaveBeenCalledWith('current-1');

    // Another surface starts a new chat: engine rebinds and the dock follows.
    useCommandCenter.setState({ chatSessionId: 'current-1' });
    await act(async () => { useCommandCenter.getState().applyCurrentChatSession('current-2'); });
    expect(lastOpts().sessionId).toBe('current-2');
    expect(switchToSession).toHaveBeenLastCalledWith('current-2');
  });

  it('defers a change while a voice turn is in flight, then applies it', async () => {
    resolveMock.mockResolvedValue('current-1');
    const switchToSession = vi.fn(async () => {});
    useCommandCenter.setState({
      switchToSession, chatSessionId: 'current-1', currentChatSessionId: null, pendingFollowSessionId: null,
    } as never);
    let busy = true;
    useVoiceMock.mockImplementation(() => ({ ...engineStub(), state: busy ? 'processing' : 'ready' }));
    await act(async () => { root.render(<VoiceHost />); });
    await act(async () => { useCommandCenter.getState().applyCurrentChatSession('current-2'); });
    // Mid-turn: still bound to the old session, dock not switched.
    expect(lastOpts().sessionId).toBe('current-1');
    expect(switchToSession).not.toHaveBeenCalled();
    busy = false;
    await act(async () => { root.render(<VoiceHost key="again" />); });
    await act(async () => { await Promise.resolve(); });
    expect(lastOpts().sessionId).toBe('current-2');
  });
});
