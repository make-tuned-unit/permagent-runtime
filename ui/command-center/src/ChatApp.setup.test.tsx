/** @vitest-environment jsdom */
import { act, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  loadProviders: vi.fn(), ensureSession: vi.fn(), loadSessionMessages: vi.fn(),
  connectSession: vi.fn(), setAgentName: vi.fn(), getIdentity: vi.fn(),
  freezeDraft: vi.fn(() => ({ text: 'Unsent draft', files: [new File(['file'], 'note.txt')], source: 'source-1', revision: 1 })),
  releaseDraft: vi.fn(), encodeDraftFiles: vi.fn(async () => [{ name: 'note.txt', mime_type: 'text/plain', data_b64: 'ZmlsZQ==' }]),
  close: vi.fn(async () => {}), emitTo: vi.fn(async (_target: string, _event: string, _payload: unknown) => {}),
  listeners: new Map<string, (event: { payload: any }) => void>(),
  state: {
    providers: [] as Array<{ name: string; isConfigured: boolean }>, providersError: false,
    currentModel: null as string | null, configRev: 0,
    chatResolvedRoute: { provider: '', model: '', source: 'unconfigured' } as { provider: string; model: string; source: string } | null,
    chatSessionRouteEvidence: false,
    chatMessages: [] as Array<{ id: string }>, chatHistoryLoaded: true,
    isStreaming: false, sessionLoadError: null as string | null,
    daemonAuthError: null as string | null,
  },
}));

vi.mock('./lib/store', () => {
  const state = Object.assign(mocks.state, {
    loadProviders: mocks.loadProviders, ensureSession: mocks.ensureSession,
    loadSessionMessages: mocks.loadSessionMessages, connectSession: mocks.connectSession,
    setAgentName: mocks.setAgentName,
    setPendingSettingsSection: vi.fn(), setActivePanel: vi.fn(),
  });
  const useCommandCenter = Object.assign(
    (selector: (value: typeof state) => unknown) => selector(state),
    { getState: () => state },
  );
  return { useCommandCenter };
});
vi.mock('./lib/api', () => ({ api: { getIdentity: mocks.getIdentity } }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: any }) => void) => {
    mocks.listeners.set(name, callback);
    return () => mocks.listeners.delete(name);
  }),
  emit: vi.fn(async () => {}),
  emitTo: (...args: [string, string, unknown]) => mocks.emitTo(...args),
}));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ close: mocks.close }) }));
vi.mock('./lib/chatDraftHandoff', () => ({
  CHAT_DRAFT_HANDOFF: 'chat_draft_handoff', CHAT_DRAFT_HANDOFF_ACK: 'chat_draft_handoff_ack',
  CHAT_DRAFT_FINAL: 'chat_draft_final', CHAT_DRAFT_FINAL_ACK: 'chat_draft_final_ack',
  decodeDraftFiles: vi.fn(() => []), encodeDraftFiles: mocks.encodeDraftFiles,
}));
vi.mock('./lib/windowChrome', () => ({
  clearPageBackgroundForVibrancy: vi.fn(), NATIVE_WINDOW_IS_OPAQUE: true, TITLEBAR_HEIGHT: 0,
}));
vi.mock('./lib/chatWindow', () => ({ trackChatGeometry: vi.fn(async () => () => {}) }));
vi.mock('./lib/voiceHandoff', () => ({ readLiveConversation: () => null, requestVoiceEnd: vi.fn() }));
vi.mock('./components/chat/MessageList', () => ({ MessageList: () => <div data-testid="message-list" /> }));
vi.mock('./components/chat/ChatInput', async () => {
  const React = await import('react');
  return { ChatInput: React.forwardRef((_props: { modelReadiness: string }, ref) => {
    React.useImperativeHandle(ref, () => ({
      addFiles: vi.fn(), restoreDraft: vi.fn(), freezeDraft: mocks.freezeDraft, releaseDraft: mocks.releaseDraft,
    }));
    return <div data-testid="chat-input" data-readiness={_props.modelReadiness} />;
  }) };
});
vi.mock('./components/chat/DropZone', () => ({ DropZone: ({ children }: { children: ReactNode }) => <>{children}</> }));
vi.mock('./components/chat/ModelPicker', () => ({ ModelPicker: () => null }));
vi.mock('./components/chat/SessionPicker', () => ({ SessionPicker: () => null }));
vi.mock('./components/inspection/InspectionPanel', () => ({ InspectionPanel: () => null }));
vi.mock('./components/awareness/AwarenessIndicator', () => ({ AwarenessIndicator: () => null }));
vi.mock('./components/awareness/PreTurnPreview', () => ({ PreTurnPreview: () => null }));
vi.mock('./components/voice/VoiceOrb', () => ({ VoiceOrb: () => null }));
vi.mock('./components/mobius/Mobius', () => ({ Mobius: () => null }));

import ChatApp from './ChatApp';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  vi.clearAllMocks();
  mocks.listeners.clear();
  mocks.emitTo.mockReset().mockResolvedValue(undefined);
  mocks.close.mockReset().mockResolvedValue(undefined);
  mocks.freezeDraft.mockReturnValue({ text: 'Unsent draft', files: [new File(['file'], 'note.txt')], source: 'source-1', revision: 1 });
  mocks.encodeDraftFiles.mockResolvedValue([{ name: 'note.txt', mime_type: 'text/plain', data_b64: 'ZmlsZQ==' }]);
  mocks.loadProviders.mockResolvedValue(undefined);
  mocks.ensureSession.mockResolvedValue(null);
  mocks.getIdentity.mockResolvedValue({ first_name: 'Ada' });
  mocks.state.providers = [];
  mocks.state.providersError = false;
  mocks.state.currentModel = null;
  mocks.state.chatResolvedRoute = { provider: '', model: '', source: 'unconfigured' };
  mocks.state.chatSessionRouteEvidence = false;
  mocks.state.chatMessages = [];
  mocks.state.chatHistoryLoaded = true;
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => { act(() => root.unmount()); host.remove(); });

it('shows setup and protects the composer in the detached window with no provider', async () => {
  await act(async () => root.render(<ChatApp />));
  expect(mocks.loadProviders).toHaveBeenCalledTimes(1);
  expect(host.querySelector('[data-testid="chat-setup-card"]')?.getAttribute('data-state')).toBe('needs-model');
  expect(host.querySelector('[data-testid="chat-input"]')?.getAttribute('data-readiness')).toBe('needs-model');
  expect(host.querySelector('[data-testid="message-list"]')).toBeNull();
});

it('shows the conversation when a provider is configured', async () => {
  mocks.state.providers = [{ name: 'openai', isConfigured: true }];
  mocks.state.chatResolvedRoute = { provider: 'openai', model: 'gpt-5', source: 'session_model' };
  await act(async () => root.render(<ChatApp />));
  expect(host.querySelector('[data-testid="message-list"]')).not.toBeNull();
  expect(host.querySelector('[data-testid="chat-setup-card"]')).toBeNull();
  expect(host.querySelector('[data-testid="chat-input"]')?.getAttribute('data-readiness')).toBe('configured');
});

it('does not treat a stale saved model name as a configured provider', async () => {
  mocks.state.currentModel = 'model-from-removed-provider';
  await act(async () => root.render(<ChatApp />));
  expect(host.querySelector('[data-testid="chat-setup-card"]')?.getAttribute('data-state')).toBe('needs-model');
  expect(host.querySelector('[data-testid="chat-input"]')?.getAttribute('data-readiness')).toBe('needs-model');
});

it('shows setup for a blank route even when the local registry entry says configured', async () => {
  mocks.state.providers = [{ name: 'local', isConfigured: true }];
  await act(async () => root.render(<ChatApp />));
  expect(host.querySelector('[data-testid="chat-setup-card"]')?.getAttribute('data-state')).toBe('needs-model');
  expect(host.querySelector('[data-testid="chat-input"]')?.getAttribute('data-readiness')).toBe('needs-model');
});

it('Return to dock waits for attachment encoding and main acknowledgement before closing', async () => {
  let finishEncoding: ((files: Array<{ name: string; mime_type: string; data_b64: string }>) => void) | undefined;
  mocks.encodeDraftFiles.mockImplementationOnce(() => new Promise(resolve => { finishEncoding = resolve; }));
  mocks.emitTo.mockImplementation(async (_target, event, payload) => {
    if (event === 'chat_draft_final') {
      mocks.listeners.get('chat_draft_final_ack')?.({ payload: { id: (payload as { id: string }).id, ok: true } });
    }
  });
  await act(async () => root.render(<ChatApp />));
  await act(async () => (host.querySelector('[aria-label="Return to dock"]') as HTMLButtonElement).click());
  expect(mocks.freezeDraft).toHaveBeenCalledOnce();
  expect(mocks.close).not.toHaveBeenCalled();
  expect(host.textContent).toContain('Saving your unsent draft and attachments');
  await act(async () => finishEncoding?.([{ name: 'note.txt', mime_type: 'text/plain', data_b64: 'ZmlsZQ==' }]));
  await vi.waitFor(() => expect(mocks.close).toHaveBeenCalledOnce());
  expect(mocks.emitTo).toHaveBeenCalledWith('main', 'chat_draft_final', expect.objectContaining({
    text: 'Unsent draft', source: 'source-1', revision: 1,
    files: [{ name: 'note.txt', mime_type: 'text/plain', data_b64: 'ZmlsZQ==' }],
  }));
});

it('keeps detached Chat and its draft visible when acknowledgement times out', async () => {
  vi.useFakeTimers();
  try {
    await act(async () => root.render(<ChatApp />));
    await act(async () => (host.querySelector('[aria-label="Return to dock"]') as HTMLButtonElement).click());
    await act(async () => { await vi.advanceTimersByTimeAsync(8100); });
    expect(mocks.close).not.toHaveBeenCalled();
    expect(mocks.releaseDraft).toHaveBeenCalledOnce();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('Your draft is still here');
    expect(host.textContent).toContain('Retry');
  } finally { vi.useRealTimers(); }
});
