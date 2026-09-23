/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createElement } from 'react';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import type { DiscussionMessage } from './decisionDiscussion';

const { createSession, sendReply, sessionEventsUrl } = vi.hoisted(() => ({
  createSession: vi.fn(), sendReply: vi.fn(), sessionEventsUrl: vi.fn(),
}));
vi.mock('./api', () => ({
  api: { createSession, sendReply, sessionEventsUrl },
  apiFetch: vi.fn(), getApiBaseUrl: () => 'http://test', loadDaemonToken: vi.fn(),
  extractText: (message: { content: { text: string }[] }) => message.content.map(c => c.text).join(''),
  extractSystemNotice: () => null, hasToolActivity: () => false,
}));
import { discussionAppContext, useDecisionDiscussion } from './decisionDiscussion';
import { useCommandCenter } from './store';
const originalChatState = useCommandCenter.getState();

class FakeSource {
  static OPEN = 1;
  static sources: FakeSource[] = [];
  readyState = 0;
  onmessage: ((event: MessageEvent<string>) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  constructor(readonly url: string) { FakeSource.sources.push(this); }
  close() { this.readyState = 2; }
  emit(data: unknown, cursor = '') {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(data), lastEventId: cursor }));
  }
  open(ids: string[] = []) {
    this.readyState = 1;
    this.emit({ type: 'ActiveRequests', request_ids: ids });
  }
}
let root: Root;
let container: HTMLDivElement;
let value: ReturnType<typeof useDecisionDiscussion>;
let sequence = 0;
let decisionId: string;
function Probe({ id }: { id: string }) {
  value = useDecisionDiscussion(id, 'Ship it?');
  return createElement('div', null, ...value.messages.map((m: DiscussionMessage) => m.text));
}
async function flush() { await act(async () => { await Promise.resolve(); }); }
function render(id: string) { act(() => root.render(createElement(Probe, { id }))); }
async function start(text = 'What are the risks?') {
  act(() => { value.setDraft(text); void value.send(); });
  await flush();
  const source = FakeSource.sources[FakeSource.sources.length - 1];
  await act(async () => source.open());
  return source;
}
function assistant(source: FakeSource, text: string) {
  source.emit({ type: 'Message', message: { role: 'assistant', content: [{ type: 'text', text }] } }, '1');
}
beforeEach(() => {
  (globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  vi.stubGlobal('EventSource', FakeSource);
  FakeSource.sources = [];
  createSession.mockReset().mockResolvedValue({ id: `session-${++sequence}` });
  sendReply.mockReset().mockResolvedValue({ request_id: 'request' });
  sessionEventsUrl.mockReset().mockImplementation(async (id: string) => `http://test/${id}`);
  decisionId = `decision-${sequence}`;
  useCommandCenter.setState({ workspaces: [{ id: 'workspace', name: 'Work', icon: '', sortOrder: 0, isDefault: true,
    layoutJson: { type: 'panel', tool: 'terminal', config: {} } }], activeWorkspaceId: 'workspace' });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  render(decisionId);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  FakeSource.sources.forEach(source => source.close());
  useCommandCenter.setState(originalChatState, true);
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('isolated decision discussion', () => {
  it('first discussion send creates exactly one correctly seeded session (ruling 6)', async () => {
    act(() => { value.setDraft('Why?'); void value.send(); void value.send(); });
    await flush();
    expect(createSession).toHaveBeenCalledTimes(1);
    expect(sendReply).not.toHaveBeenCalled();
    await act(async () => FakeSource.sources[0].open());
    expect(sendReply).toHaveBeenCalledTimes(1);
    expect(sendReply).toHaveBeenCalledWith(`session-${sequence}`,
      'I\'d like to talk through this decision: "Ship it?".\n\nWhy?', undefined,
      { current_tab: 'terminal', active_panel: 'decision-review', view_state: { discuss_decision_id: decisionId } });
    expect(value.status).toBe('sending');
    act(() => FakeSource.sources[0].emit({ type: 'Finish' }));
    act(() => { value.setDraft('And cost?'); });
    await act(async () => value.send());
    expect(sendReply.mock.calls[1][1]).toBe('And cost?');
    expect(sendReply.mock.calls[1][3]).toEqual({ current_tab: 'terminal', active_panel: 'decision-review' });
    expect(createSession).toHaveBeenCalledTimes(1);
  });

  it('returning to a decision restores its discussion without changing main chat (ruling 6)', async () => {
    useCommandCenter.setState({
      chatSessionId: 'main-session',
      chatMessages: [{ id: 'main-message', role: 'assistant', content: 'Main chat is busy', timestamp: '2026-09-22' }],
      isStreaming: true, connectionStatus: 'connected', activePanel: 'settings', chatDockOpen: true,
    });
    const before = useCommandCenter.getState();
    const mainChat = () => {
      const s = useCommandCenter.getState();
      return [s.chatSessionId, s.chatMessages, s.isStreaming, s.connectionStatus,
        s._eventSource, s.activePanel, s.chatDockOpen, s.chatWindowOpen];
    };
    const fields = mainChat();
    const source = await start();
    act(() => { assistant(source, 'Consider the risk.'); source.emit({ type: 'Finish' }); value.setDraft('Saved draft'); });
    render('another-decision');
    expect(value.messages).toEqual([]);
    act(() => root.unmount());
    root = createRoot(container);
    render(decisionId);
    expect(value.sessionId).toBe(`session-${sequence}`);
    expect(value.messages.map(m => m.text)).toEqual(['What are the risks?', 'Consider the risk.']);
    expect(value.draft).toBe('Saved draft');
    expect(createSession).toHaveBeenCalledTimes(1);
    expect(mainChat()).toEqual(fields);
    expect(useCommandCenter.getState()).toBe(before);
  });

  it('late messages never appear under another decision (ruling 6)', async () => {
    const source = await start();
    render('late-other');
    act(() => assistant(source, 'Only the original decision'));
    expect(value.messages).toEqual([]);
    expect(container.textContent).not.toContain('Only the original');
    render(decisionId);
    await flush();
    const reconnected = FakeSource.sources[FakeSource.sources.length - 1];
    await act(async () => reconnected.open(['request']));
    act(() => assistant(reconnected, 'Only the original decision'));
    expect(value.messages[1].text).toBe('Only the original decision');
  });

  it('session creation failure preserves the draft and permits retry (ruling 7)', async () => {
    createSession.mockRejectedValueOnce(new Error('daemon unavailable'));
    act(() => value.setDraft('Keep this question'));
    await act(async () => value.send());
    expect(value.status).toBe('error');
    expect(value.error).toBe('daemon unavailable');
    expect(value.draft).toBe('Keep this question');
    act(() => { void value.retry(); });
    await flush();
    await act(async () => FakeSource.sources[0].open());
    expect(createSession).toHaveBeenCalledTimes(2);
    expect(value.draft).toBe('');
    expect(value.messages[0].text).toBe('Keep this question');
  });

  it('connection failure retries the same session and preserves the draft (ruling 7)', async () => {
    act(() => { value.setDraft('Preserve me'); void value.send(); });
    await flush();
    const old = FakeSource.sources[0];
    for (const delay of [500, 1000, 2000, null]) {
      await act(async () => FakeSource.sources[FakeSource.sources.length - 1].onerror?.(
        new ErrorEvent('error', { message: 'daemon refused stream' })));
      if (delay !== null) await act(async () => { await vi.advanceTimersByTimeAsync(delay); });
    }
    expect(value.error).toBe('daemon refused stream');
    expect(value.draft).toBe('Preserve me');
    expect(sendReply).not.toHaveBeenCalled();
    act(() => { void value.retry(); });
    await flush();
    await act(async () => FakeSource.sources[4].open());
    expect(createSession).toHaveBeenCalledTimes(1);
    expect(sendReply).toHaveBeenCalledTimes(1);
    act(() => assistant(old, 'stale transport'));
    expect(value.messages[1].text).toBe('');
  });

  it('a rejected reply preserves the draft for retry in the same session (ruling 7)', async () => {
    sendReply.mockRejectedValueOnce(new Error('reply unavailable'));
    await start('Keep this text');
    expect(value.status).toBe('error');
    expect(value.error).toBe('reply unavailable');
    expect(value.draft).toBe('Keep this text');
    expect(value.messages).toEqual([]);
    await act(async () => value.retry());
    expect(createSession).toHaveBeenCalledTimes(1);
    expect(sendReply).toHaveBeenCalledTimes(2);
    expect(value.messages[0].text).toBe('Keep this text');
  });

  it('reattaches an accepted turn without resending it and resumes its cursor (rulings 6, 7)', async () => {
    const source = await start();
    act(() => assistant(source, 'First'));
    await act(async () => source.onerror?.(new Event('error')));
    await act(async () => { await vi.advanceTimersByTimeAsync(500); });
    await act(async () => FakeSource.sources[1].open(['request']));
    expect(sessionEventsUrl).toHaveBeenLastCalledWith(`session-${sequence}`, '1');
    expect(sendReply).toHaveBeenCalledTimes(1);
    act(() => { assistant(FakeSource.sources[1], ' second'); FakeSource.sources[1].emit({ type: 'Finish' }); });
    expect(value.messages[1].text).toBe('First second');
    expect(value.status).toBe('ready');
  });

  it('unmount closes the stream and remount reconnects the same session from its cursor (ruling 6)', async () => {
    const source = await start();
    act(() => { assistant(source, 'Saved answer'); source.emit({ type: 'Finish' }, 'saved-cursor'); value.setDraft('Saved draft'); });
    const sessionId = value.sessionId;
    act(() => root.unmount());
    expect(source.readyState).toBe(2);
    root = createRoot(container);
    render(decisionId);
    await flush();
    expect(value.sessionId).toBe(sessionId);
    expect(value.draft).toBe('Saved draft');
    expect(value.messages[1].text).toBe('Saved answer');
    expect(createSession).toHaveBeenCalledTimes(1);
    expect(FakeSource.sources).toHaveLength(2);
    expect(sessionEventsUrl).toHaveBeenLastCalledWith(sessionId, 'saved-cursor');
    await act(async () => FakeSource.sources[1].open());
    expect(sendReply).toHaveBeenCalledTimes(1);
  });

  it('two transient failures recover automatically and reset retries without a duplicate turn (ruling 7)', async () => {
    await start();
    for (const delay of [500, 1000]) {
      await act(async () => FakeSource.sources[FakeSource.sources.length - 1].onerror?.(new Event('error')));
      expect(value.status).not.toBe('error');
      await act(async () => { await vi.advanceTimersByTimeAsync(delay); });
    }
    await act(async () => FakeSource.sources[2].open());
    expect(value.status).toBe('ready');
    expect(value.error).toBeNull();
    expect(sendReply).toHaveBeenCalledTimes(1);
    await act(async () => FakeSource.sources[2].onerror?.(new Event('error')));
    await act(async () => { await vi.advanceTimersByTimeAsync(500); });
    expect(FakeSource.sources).toHaveLength(4);
    await act(async () => FakeSource.sources[3].open());
    expect(value.status).toBe('ready');
    expect(sendReply).toHaveBeenCalledTimes(1);
  });

  it('four connection failures surface an inline error after bounded retries (ruling 7)', async () => {
    await start();
    for (const delay of [500, 1000, 2000, null]) {
      await act(async () => FakeSource.sources[FakeSource.sources.length - 1].onerror?.(new Event('error')));
      if (delay !== null) {
        expect(value.status).not.toBe('error');
        await act(async () => { await vi.advanceTimersByTimeAsync(delay); });
      }
    }
    expect(value.status).toBe('error');
    expect(value.error).toBe('Could not connect to this decision discussion');
    await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
    expect(FakeSource.sources).toHaveLength(4);
    expect(sendReply).toHaveBeenCalledTimes(1);
  });

  it('unmount cancels a scheduled reconnect (ruling 6)', async () => {
    const source = await start();
    await act(async () => source.onerror?.(new Event('error')));
    render('unmounted-retry');
    await act(async () => { await vi.advanceTimersByTimeAsync(500); });
    expect(FakeSource.sources).toHaveLength(1);
    expect(source.readyState).toBe(2);
  });

  it('missing session id preserves the draft with an explicit creation error', async () => {
    createSession.mockResolvedValueOnce({});
    act(() => value.setDraft('Keep me'));
    await act(async () => value.send());
    expect(value.status).toBe('error');
    expect(value.error).toBe('Discussion session creation returned no session id');
    expect(value.draft).toBe('Keep me');
    expect(sessionEventsUrl).not.toHaveBeenCalled();
    expect(sendReply).not.toHaveBeenCalled();
  });

  it('discussion context follows the active layout primary tool and seeds only the first turn', () => {
    const state: Parameters<typeof discussionAppContext>[0] = {
      activeWorkspaceId: 'active',
      workspaces: [
        { id: 'other', layoutJson: { type: 'panel', tool: 'dashboard', config: {} } },
        { id: 'active', layoutJson: { type: 'split', direction: 'horizontal', sizes: [50, 50], children: [
          { type: 'split', direction: 'vertical', sizes: [100], children: [
            { type: 'panel', tool: 'terminal', config: {} },
          ] },
          { type: 'panel', tool: 'dashboard', config: {} },
        ] } },
      ],
    };
    expect(discussionAppContext(state, decisionId, false)).toEqual({
      current_tab: 'terminal', active_panel: 'decision-review', view_state: { discuss_decision_id: decisionId },
    });
    expect(discussionAppContext(state, decisionId, true)).toEqual({ current_tab: 'terminal', active_panel: 'decision-review' });
    // No nameable workspace tool must never cost the daemon the decision id.
    expect(discussionAppContext({ ...state, activeWorkspaceId: null }, decisionId, false)).toEqual({
      current_tab: 'dashboard', active_panel: 'decision-review', view_state: { discuss_decision_id: decisionId },
    });
  });

});
