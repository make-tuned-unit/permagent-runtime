/** @vitest-environment jsdom */
import { act, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { chatSetupState, modelSetupReadiness } from './chatSetupState';

const nativeMocks = vi.hoisted(() => ({
  emit: vi.fn(), getByLabel: vi.fn(), show: vi.fn(), setFocus: vi.fn(),
}));
vi.mock('@tauri-apps/api/event', () => ({ emit: nativeMocks.emit }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ WebviewWindow: { getByLabel: nativeMocks.getByLabel } }));

const mocks = vi.hoisted(() => ({
  loadProviders: vi.fn(),
  ensureSession: vi.fn(),
  loadSessionMessages: vi.fn(),
  connectSession: vi.fn(),
  disconnectSession: vi.fn(),
  setAgentName: vi.fn(),
  setActivePanel: vi.fn(),
  setPendingSettingsSection: vi.fn(),
  getIdentity: vi.fn(),
  state: {
    providers: [] as Array<{ name: string; isConfigured: boolean }>, providersError: false, currentModel: null as string | null,
    chatResolvedRoute: { provider: '', model: '', source: 'unconfigured' } as { provider: string; model: string; source: string } | null,
    chatSessionRouteEvidence: false,
    configRev: 0, chatMessages: [] as Array<{ id: string }>, chatHistoryLoaded: true,
    isStreaming: false, sessionLoadError: null as string | null, daemonAuthError: null as string | null,
    voiceConversation: null, worldConversationActive: false, chatWindowOpen: false,
    pendingChatFiles: null, identityRev: 0,
  },
}));

vi.mock('../../lib/store', () => {
  const state = Object.assign(mocks.state, {
    loadProviders: mocks.loadProviders,
    ensureSession: mocks.ensureSession,
    loadSessionMessages: mocks.loadSessionMessages,
    connectSession: mocks.connectSession,
    disconnectSession: mocks.disconnectSession,
    setAgentName: mocks.setAgentName,
    setActivePanel: mocks.setActivePanel,
    setPendingSettingsSection: mocks.setPendingSettingsSection,
    takeChatFiles: () => null,
  });
  const useCommandCenter = Object.assign(
    (selector: (value: typeof state) => unknown) => selector(state),
    { getState: () => state },
  );
  return { useCommandCenter };
});
vi.mock('../../lib/api', () => ({ api: { getIdentity: mocks.getIdentity } }));
vi.mock('./MessageList', () => ({ MessageList: () => <div data-testid="message-list" /> }));
vi.mock('./ChatInput', () => ({ ChatInput: ({ modelReadiness }: { modelReadiness: string }) => <div data-testid="chat-input" data-readiness={modelReadiness} /> }));
vi.mock('./SkillPromptBanner', () => ({ SkillPromptBanner: () => null }));
vi.mock('./RoleRoutingPrompt', () => ({ RoleRoutingPrompt: () => null }));
vi.mock('./ModelPicker', () => ({ ModelPicker: () => null }));
vi.mock('./SessionPicker', () => ({ SessionPicker: () => null }));
vi.mock('./DiscussNotice', () => ({ DiscussNotice: () => null }));
vi.mock('./DropZone', () => ({ DropZone: ({ children }: { children: ReactNode }) => <>{children}</> }));
vi.mock('../voice/VoiceOrb', () => ({ VoiceOrb: () => null }));

import { ChatView } from './ChatView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
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
  mocks.state.sessionLoadError = null;
  mocks.state.daemonAuthError = null;
  nativeMocks.emit.mockResolvedValue(undefined);
  nativeMocks.show.mockResolvedValue(undefined);
  nativeMocks.setFocus.mockResolvedValue(undefined);
  nativeMocks.getByLabel.mockResolvedValue({ show: nativeMocks.show, setFocus: nativeMocks.setFocus });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount()); host.remove();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  window.history.replaceState(null, '', '/');
});

it('treats a successful empty registry as setup needed and routes to Models without sending', async () => {
  await act(async () => root.render(<ChatView />));
  expect(mocks.loadProviders).toHaveBeenCalledTimes(1);
  expect(host.querySelector('[data-testid="chat-setup-card"]')?.getAttribute('data-state')).toBe('needs-model');
  expect(host.textContent).toContain('Chat needs a selected model before it can reply');
  expect(host.querySelector('[data-testid="chat-input"]')?.getAttribute('data-readiness')).toBe('needs-model');
  expect(host.querySelector('[data-testid="message-list"]')).toBeNull();
  const open = [...host.querySelectorAll('button')].find(button => button.textContent === 'Open model setup')!;
  act(() => open.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  expect(mocks.setPendingSettingsSection).toHaveBeenCalledWith('models');
  expect(mocks.setActivePanel).toHaveBeenCalledWith('settings');
});

it('keeps a configured provider on the normal conversation surface', async () => {
  mocks.state.providers = [{ name: 'openai', isConfigured: true }];
  mocks.state.chatResolvedRoute = { provider: 'openai', model: 'gpt-5', source: 'session_model' };
  await act(async () => root.render(<ChatView />));
  expect(host.querySelector('[data-testid="message-list"]')).not.toBeNull();
  expect(host.querySelector('[data-testid="chat-setup-card"]')).toBeNull();
  expect(host.querySelector('[data-testid="chat-input"]')?.getAttribute('data-readiness')).toBe('configured');
});

it('opens Models in the main window from detached Chat', async () => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  window.history.replaceState(null, '', '/?view=chat');
  await act(async () => root.render(<ChatView />));
  const open = [...host.querySelectorAll('button')].find(button => button.textContent === 'Open model setup')!;
  await act(async () => { open.click(); await Promise.resolve(); });
  await vi.waitFor(() => expect(nativeMocks.setFocus).toHaveBeenCalledOnce());
  expect(nativeMocks.emit).toHaveBeenCalledWith('app_navigate', {
    tool_type: 'settings', panel_type: 'overlay', section: 'models',
  });
  expect(nativeMocks.getByLabel).toHaveBeenCalledWith('main');
  expect(nativeMocks.show).toHaveBeenCalledOnce();
  expect(nativeMocks.setFocus).toHaveBeenCalledOnce();
  expect(mocks.setActivePanel).not.toHaveBeenCalled();
});

it('keeps configured or existing chats out of the setup state', () => {
  const readiness = modelSetupReadiness({ providers: [{ name: 'openai', isConfigured: true }] as never, providerCheckKnown: true, providersError: false, chatRoute: { provider: 'openai', model: 'gpt-5', source: 'default' }, sessionRouteEvidence: false, hasConversation: false });
  expect(readiness).toBe('configured');
  expect(chatSetupState({ modelReadiness: 'needs-model', historyLoaded: true, messageCount: 1, streaming: false, sessionLoadError: null, daemonAuthError: null, voiceActive: false })).toBeNull();
  expect(modelSetupReadiness({ providers: [], providerCheckKnown: true, providersError: false, chatRoute: { provider: '', model: '', source: 'unconfigured' }, sessionRouteEvidence: false, hasConversation: false })).toBe('needs-model');
});

it('offers retry without claiming a provider is missing after a failed read', async () => {
  mocks.loadProviders.mockImplementation(async () => { mocks.state.providersError = true; });
  await act(async () => root.render(<ChatView />));
  const card = host.querySelector('[data-testid="chat-setup-card"]');
  expect(card?.getAttribute('data-state')).toBe('unavailable');
  expect(card?.textContent).toContain('could not confirm');
  expect(card?.textContent).not.toContain('Chat needs a selected model');
  expect([...host.querySelectorAll('button')].some(button => button.textContent === 'Retry check')).toBe(true);
});

it('distinguishes unknown provider status from a known missing model', () => {
  const missingRoute = { provider: '', model: '', source: 'unconfigured' };
  const base = { providers: [], chatRoute: missingRoute, sessionRouteEvidence: false, hasConversation: false };
  expect(modelSetupReadiness({ ...base, providerCheckKnown: false, providersError: false })).toBe('checking');
  expect(modelSetupReadiness({ ...base, providerCheckKnown: true, providersError: true })).toBe('unavailable');
  expect(modelSetupReadiness({ ...base, providerCheckKnown: true, providersError: false })).toBe('needs-model');
});

it('requires the effective Chat route, not any configured registry entry', () => {
  const local = { name: 'local', isConfigured: true } as never;
  const hosted = { name: 'openai', isConfigured: true } as never;
  const evaluate = (providers: never[], chatRoute: { provider: string; model: string; source: string } | null,
    sessionRouteEvidence = false, hasConversation = false) => modelSetupReadiness({
      providers, chatRoute, providerCheckKnown: true, providersError: false,
      sessionRouteEvidence, hasConversation,
    });
  const noRoute = { provider: '', model: '', source: 'unconfigured' };
  // The blank registry still marks `local` configured, without a Chat model.
  expect(evaluate([local], noRoute)).toBe('needs-model');
  expect(evaluate([local, hosted], noRoute)).toBe('needs-model');
  expect(evaluate([hosted], null)).toBe('unavailable'); // older daemon
  expect(evaluate([hosted], { provider: 'openai', model: 'gpt-5', source: 'default' })).toBe('configured');
  expect(evaluate([hosted], { provider: 'openai', model: 'gpt-5', source: 'session_model' })).toBe('configured');
  expect(evaluate([hosted], { provider: 'anthropic', model: 'claude', source: 'configured' })).toBe('unavailable');
  expect(evaluate([local], noRoute, true)).toBe('unavailable'); // saved session may restore
  expect(evaluate([local], noRoute, false, true)).toBe('unavailable'); // existing conversation
});
