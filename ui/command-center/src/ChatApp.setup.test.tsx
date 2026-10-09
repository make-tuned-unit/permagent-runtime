/** @vitest-environment jsdom */
import { act, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  loadProviders: vi.fn(), ensureSession: vi.fn(), loadSessionMessages: vi.fn(),
  connectSession: vi.fn(), setAgentName: vi.fn(), getIdentity: vi.fn(),
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
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));
vi.mock('./lib/windowChrome', () => ({
  clearPageBackgroundForVibrancy: vi.fn(), NATIVE_WINDOW_IS_OPAQUE: true, TITLEBAR_HEIGHT: 0,
}));
vi.mock('./lib/chatWindow', () => ({ trackChatGeometry: vi.fn(async () => () => {}) }));
vi.mock('./lib/voiceHandoff', () => ({ readLiveConversation: () => null, requestVoiceEnd: vi.fn() }));
vi.mock('./components/chat/MessageList', () => ({ MessageList: () => <div data-testid="message-list" /> }));
vi.mock('./components/chat/ChatInput', () => ({ ChatInput: ({ modelReadiness }: { modelReadiness: string }) => <div data-testid="chat-input" data-readiness={modelReadiness} /> }));
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
