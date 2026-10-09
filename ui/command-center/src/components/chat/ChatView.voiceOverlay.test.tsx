/**
 * @vitest-environment jsdom
 *
 * Conversation mode, and decisions staying out of chat.
 *
 * The 2026-09-18 report was that the orb's `SAY "HEY HENRY"` state word drew
 * through the pending-decision cards; the fix of the day put the decisions
 * inside the orb's own flex column. On 2026-09-20 the user asked for those
 * cards out of the chat surface altogether — "i dont like how the decisions
 * are shown in the chat window ... i dont like the designs and buttons" — so
 * the overlap cannot happen for the simplest reason available: chat does not
 * render decisions at all. They live in the Decision Inbox on Home.
 *
 * This pins both halves: the orb still takes the surface over and still says
 * its own words, and no decision card or approval button appears in chat.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  ensureSession: vi.fn(),
  loadSessionMessages: vi.fn(),
  connectSession: vi.fn(),
  disconnectSession: vi.fn(),
  setAgentName: vi.fn(),
  getIdentity: vi.fn(),
  loadProviders: vi.fn(),
  exit: vi.fn(),
}));

vi.mock('../../lib/store', () => {
  const state = {
    ensureSession: mocks.ensureSession,
    loadSessionMessages: mocks.loadSessionMessages,
    connectSession: mocks.connectSession,
    disconnectSession: mocks.disconnectSession,
    setAgentName: mocks.setAgentName,
    loadProviders: mocks.loadProviders,
    providers: [],
    providersError: false,
    currentModel: null,
    configRev: 0,
    chatMessages: [],
    chatHistoryLoaded: true,
    isStreaming: false,
    sessionLoadError: null,
    daemonAuthError: null,
    identityRev: 0,
    agentName: 'Henry',
    sessions: [],
    chatSessionId: null,
    pendingChatFiles: null,
    takeChatFiles: () => null,
    // Hands-free is live and the chat is docked, so this window owns the orb.
    voiceConversation: {
      state: 'ready',
      getPlaybackAnalyser: () => null,
      getMicAnalyser: () => null,
      exit: mocks.exit,
      wakeHint: 'Say "Hey Henry"',
      teachWord: null,
    },
    chatWindowOpen: false,
  };
  const useCommandCenter = Object.assign(
    (selector: (value: typeof state) => unknown) => selector(state),
    { getState: () => state },
  );
  return { useCommandCenter };
});
vi.mock('../../lib/api', () => ({
  api: { getIdentity: mocks.getIdentity, createSession: vi.fn() },
}));
vi.mock('./MessageList', () => ({ MessageList: () => null }));
vi.mock('./ChatInput', () => ({ ChatInput: () => null }));
vi.mock('./SkillPromptBanner', () => ({ SkillPromptBanner: () => null }));
vi.mock('./RoleRoutingPrompt', () => ({ RoleRoutingPrompt: () => null }));
vi.mock('./ModelPicker', () => ({ ModelPicker: () => null }));
vi.mock('./SessionPicker', () => ({ SessionPicker: () => null }));
vi.mock('./DiscussNotice', () => ({ DiscussNotice: () => null }));
import { ChatView } from './ChatView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  mocks.getIdentity.mockRejectedValue(new Error('offline'));
  mocks.loadProviders.mockResolvedValue(undefined);
  mocks.ensureSession.mockResolvedValue(null);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

function render() {
  act(() => { root.render(<ChatView />); });
}

it('takes the surface over and says its own words', () => {
  render();
  const takeover = container.querySelector<HTMLElement>('[data-testid="voice-orb-takeover"]');
  expect(takeover).toBeTruthy();
  expect(takeover!.textContent).toContain('Say "Hey Henry"');
  expect(takeover!.textContent).toContain('click anywhere to end the conversation');
});

it('renders no decision card and no approval button anywhere in chat', () => {
  render();
  expect(container.querySelector('[data-testid="chat-pending-decisions"]')).toBeNull();
  expect(container.querySelector('[data-testid="voice-orb-footer"]')).toBeNull();
  for (const word of ['Approve', 'Reject', 'Add note', 'Decision Inbox']) {
    expect(container.textContent, word).not.toContain(word);
  }
});
