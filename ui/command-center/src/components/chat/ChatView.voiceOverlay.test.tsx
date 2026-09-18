/**
 * @vitest-environment jsdom
 *
 * Conversation-mode layering.
 *
 * Reported 2026-09-18: with the orb up, the cyan `SAY "HEY HENRY"` state word
 * and its `click anywhere to end the conversation` caption were drawn straight
 * through the pending-decision text ("IREN looks stretched — consider taking
 * the gain"). ChatView floated the decisions in their own
 * `position: absolute; bottom` box at `zIndex: 70` over the orb's `inset: 0`
 * takeover at `zIndex: 60` — two layers with no layout relationship, the upper
 * one transparent, and the orb centring its labels in the FULL surface height.
 *
 * jsdom has no layout engine, so this cannot assert "the boxes do not
 * intersect". What it CAN assert is the structural invariant that makes the
 * intersection impossible: the decisions render inside the orb's own flex
 * column, and nothing between them is taken out of flow. Two absolutely
 * positioned siblings — the shape of the bug — fail that.
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
  exit: vi.fn(),
}));

vi.mock('../../lib/store', () => {
  const state = {
    ensureSession: mocks.ensureSession,
    loadSessionMessages: mocks.loadSessionMessages,
    connectSession: mocks.connectSession,
    disconnectSession: mocks.disconnectSession,
    setAgentName: mocks.setAgentName,
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
// Stand-in for the real card list: this test is about WHERE ChatView puts the
// decisions, not what a decision row renders.
vi.mock('./ChatPendingDecisions', () => ({
  ChatPendingDecisions: () => (
    <div data-testid="chat-pending-decisions">IREN looks stretched — consider taking the gain</div>
  ),
}));

import { ChatView } from './ChatView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  mocks.getIdentity.mockRejectedValue(new Error('offline'));
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

/** Every element from `el` up to (but not including) `stop`. */
function chainTo(el: HTMLElement, stop: HTMLElement): HTMLElement[] {
  const out: HTMLElement[] = [];
  for (let node: HTMLElement = el; node && node !== stop; node = node.parentElement!) out.push(node);
  return out;
}

function outOfFlow(el: HTMLElement): boolean {
  return el.style.position === 'absolute' || el.style.position === 'fixed';
}

/** Deepest element containing the whole of `text`. */
function deepestWith(text: string): HTMLElement {
  const all = Array.from(container.querySelectorAll<HTMLElement>('*'))
    .filter(el => el.textContent?.includes(text));
  return all[all.length - 1];
}

it('lays the orb labels and the decisions out in one flow, not as two layers', () => {
  render();
  // Deliberately found by their COPY, not by a test id: the invariant is about
  // the two things the user saw overlap, and it must hold however they are
  // marked up.
  const caption = deepestWith('click anywhere to end the conversation');
  const decisions = deepestWith('IREN looks stretched');
  expect(caption).toBeTruthy();
  expect(decisions).toBeTruthy();

  // Walk both up to the first element that holds both.
  let shared: HTMLElement = decisions;
  while (!shared.contains(caption)) shared = shared.parentElement!;

  // Neither branch is taken out of flow below that point, so the shared
  // container's layout — one flex column — is what separates them. Before the
  // fix the decisions hung off ChatView in their own `position: absolute`
  // box while the orb centred the caption in the full height, and the two
  // simply rendered through each other.
  for (const el of chainTo(decisions, shared)) expect(outOfFlow(el)).toBe(false);
  for (const el of chainTo(caption, shared)) expect(outOfFlow(el)).toBe(false);
  // And the shared container really is that column, not the chat surface.
  expect(shared.getAttribute('data-testid')).toBe('voice-orb-takeover');
});

it('renders exactly one pending-decisions layer while the orb is up', () => {
  render();
  // The inline copy is suppressed under the takeover; the footer copy is the
  // only one. Two simultaneous copies is the duplicate-layer bug by another
  // name (and would double every approval button).
  expect(container.querySelectorAll('[data-testid="chat-pending-decisions"]').length).toBe(1);
});

it('keeps the orb state word and its caption out of the decisions box', () => {
  render();
  const orbBlock = container.querySelector<HTMLElement>('[data-testid="voice-orb-takeover"]')!
    .firstElementChild as HTMLElement;
  const footer = container.querySelector<HTMLElement>('[data-testid="voice-orb-footer"]')!;
  // Separate flex children: the wake hint and the caption live in the orb
  // block, the decisions in the footer, and neither subtree contains the other.
  expect(orbBlock.textContent).toContain('Say "Hey Henry"');
  expect(orbBlock.textContent).toContain('click anywhere to end the conversation');
  expect(orbBlock.contains(footer)).toBe(false);
  expect(footer.textContent).toContain('IREN looks stretched');
  expect(footer.textContent).not.toContain('Hey Henry');
});
