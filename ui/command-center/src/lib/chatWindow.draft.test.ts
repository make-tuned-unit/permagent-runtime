/** @vitest-environment jsdom */
import { beforeEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, Set<(event: { payload: any }) => void>>();
  const state = {
    chatDraft: { text: 'Keep my private draft', files: [new File(['note'], 'review.txt')] },
    chatWindowOpen: false, chatDockOpen: true, chatSessionId: 'session-1',
    ensureSession: vi.fn(),
  };
  return {
    listeners, state, options: [] as Array<{ url: string }>,
    close: vi.fn(async () => {}), emitTo: vi.fn(async (_target: string, _event: string, _payload: unknown) => {}),
    encode: vi.fn(async () => [{ name: 'review.txt', mime_type: 'text/plain', data_b64: 'bm90ZQ==' }]),
  };
});

vi.mock('./store', () => ({ useCommandCenter: {
  getState: () => mocks.state,
  setState: (patch: Partial<typeof mocks.state>) => Object.assign(mocks.state, patch),
} }));
vi.mock('./chatDraftHandoff', () => ({
  CHAT_DRAFT_HANDOFF: 'chat_draft_handoff', CHAT_DRAFT_HANDOFF_ACK: 'chat_draft_handoff_ack',
  encodeDraftFiles: mocks.encode,
}));
vi.mock('@tauri-apps/api/window', () => ({ Effect: { Sidebar: 'sidebar' }, EffectState: { FollowsWindowActiveState: 'active' } }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ WebviewWindow: class {
  constructor(_label: string, options: { url: string }) {
    mocks.options.push(options);
    const marker = new URL(options.url, 'http://localhost').searchParams.get('draft_handoff');
    queueMicrotask(() => dispatch('chat_ready', { draft_handoff: marker }));
  }
  once = vi.fn(async () => () => {});
  close = mocks.close;
} }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, callback: (value: { payload: any }) => void) => {
    let set = mocks.listeners.get(event);
    if (!set) { set = new Set(); mocks.listeners.set(event, set); }
    set.add(callback);
    return () => set?.delete(callback);
  }),
  emitTo: (...args: [string, string, unknown]) => mocks.emitTo(...args),
}));
vi.mock('./windowChrome', () => ({
  applyWindowTransparency: vi.fn(), reinsetTrafficLights: vi.fn(),
  NATIVE_WINDOW_IS_OPAQUE: true, OVERLAY_TITLEBAR: {},
}));

import { createChatWindow } from './chatWindow';

function dispatch(event: string, payload: unknown) {
  for (const callback of mocks.listeners.get(event) ?? []) callback({ payload });
}

beforeEach(() => {
  mocks.listeners.clear();
  mocks.options.length = 0;
  mocks.close.mockClear();
  mocks.emitTo.mockReset();
  mocks.encode.mockClear();
  mocks.state.chatDraft = { text: 'Keep my private draft', files: [new File(['note'], 'review.txt')] };
  mocks.state.chatWindowOpen = false;
  mocks.state.chatDockOpen = true;
  mocks.state.chatSessionId = 'session-1';
  localStorage.clear();
  localStorage.setItem('permagent-chat-setup-intent', 'Help me set up Tailscale');
});

it('hands off text and files only after detached Chat is ready and acknowledges them', async () => {
  mocks.emitTo.mockImplementation(async (_target, event, payload) => {
    if (event === 'chat_draft_handoff') dispatch('chat_draft_handoff_ack', { id: (payload as { id: string }).id });
  });
  const first = createChatWindow('silver');
  expect(mocks.state.chatDockOpen).toBe(false);
  expect(createChatWindow('silver')).toBe(first);
  await first;
  expect(mocks.options).toHaveLength(1);
  const url = mocks.options[0].url;
  expect(url).toContain('draft_handoff=');
  expect(url).not.toContain('Keep');
  expect(url).not.toContain('review.txt');
  expect(mocks.emitTo).toHaveBeenCalledWith('chat', 'chat_draft_handoff', expect.objectContaining({
    text: 'Keep my private draft', pendingSetup: expect.objectContaining({ text: 'Help me set up Tailscale' }),
    files: [{ name: 'review.txt', mime_type: 'text/plain', data_b64: 'bm90ZQ==' }],
  }));
  expect(mocks.state.chatWindowOpen).toBe(true);
  expect(mocks.state.chatDockOpen).toBe(false);
});

it('restores the dock and original draft if native handoff fails', async () => {
  mocks.emitTo.mockRejectedValueOnce(new Error('event unavailable'));
  await expect(createChatWindow('silver')).rejects.toThrow('event unavailable');
  expect(mocks.close).toHaveBeenCalledOnce();
  expect(mocks.state.chatWindowOpen).toBe(false);
  expect(mocks.state.chatDockOpen).toBe(true);
  expect(mocks.state.chatDraft.text).toBe('Keep my private draft');
  expect(mocks.state.chatDraft.files[0].name).toBe('review.txt');
  expect(localStorage.getItem('permagent-chat-setup-intent')).toBe('Help me set up Tailscale');
});
