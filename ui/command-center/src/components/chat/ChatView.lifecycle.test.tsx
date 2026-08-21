/** @vitest-environment jsdom */
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
  loadSessions: vi.fn(),
  switchToSession: vi.fn(),
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
    loadSessions: mocks.loadSessions,
    switchToSession: mocks.switchToSession,
    chatSessionId: null,
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
vi.mock('./ModelPicker', () => ({ ModelPicker: () => null }));
vi.mock('./ChatPendingDecisions', () => ({ ChatPendingDecisions: () => null }));

import { ChatView } from './ChatView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  Object.values(mocks).forEach(mock => mock.mockReset());
  mocks.getIdentity.mockRejectedValue(new Error('offline'));
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

it('does not load history or open SSE after unmount while session creation is pending', async () => {
  const pendingSession = deferred<string>();
  mocks.ensureSession.mockReturnValue(pendingSession.promise);

  await act(async () => root.render(<ChatView />));
  act(() => root.unmount());
  await act(async () => pendingSession.resolve('session-1'));

  expect(mocks.disconnectSession).toHaveBeenCalledTimes(1);
  expect(mocks.loadSessionMessages).not.toHaveBeenCalled();
  expect(mocks.connectSession).not.toHaveBeenCalled();
});

it('renders the shared session picker in the docked chat header', async () => {
  mocks.ensureSession.mockResolvedValue(undefined);

  await act(async () => root.render(<ChatView />));

  expect(container.querySelector('[data-testid="session-picker"]')).not.toBeNull();
  expect(container.querySelector('[aria-label="Choose chat session"]')?.textContent).toContain('Henry');
});
