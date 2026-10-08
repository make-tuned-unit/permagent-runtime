/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useCommandCenter, type ChatMessage } from '../../../lib/store';
import { ForumConversation } from './ForumConversation';

vi.mock('../../chat/MessageBubble', () => ({
  MessageBubble: ({ message }: { message: ChatMessage }) => <article>{message.content}</article>,
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
let previous: ReturnType<typeof useCommandCenter.getState>;
const ensureSession = vi.fn();
const connectSession = vi.fn();
const sendMessage = vi.fn();
const stopStreaming = vi.fn();
const openChatDock = vi.fn<[], void>();

beforeEach(() => {
  previous = useCommandCenter.getState();
  vi.clearAllMocks();
  // Exercise the real dock action; opening the preview must not start its own
  // session or submit a prompt. ChatView owns the normal connection lifecycle.
  openChatDock.mockImplementation(previous.openChatDock);
  useCommandCenter.setState({
    chatSessionId: 'existing-conversation', chatMessages: [],
    connectionStatus: 'disconnected', chatWindowOpen: false, chatDockOpen: false,
    isStreaming: false, _activeRequestId: null,
    ensureSession, connectSession, sendMessage, stopStreaming, openChatDock,
  });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  useCommandCenter.setState(previous);
});

const button = (label: string) => [...host.querySelectorAll('button')]
  .find(element => element.textContent?.includes(label))!;

it('opens the current shared chat through the dock without sending or creating a session', () => {
  act(() => root.render(<ForumConversation />));
  expect(host.textContent).toContain('No messages to show here yet.');
  expect(host.querySelector('[role="status"]')?.textContent).toContain('Open it to continue your conversation.');
  expect(openChatDock).not.toHaveBeenCalled();
  expect(useCommandCenter.getState().chatDockOpen).toBe(false);

  act(() => button('Open shared chat').click());

  expect(openChatDock).toHaveBeenCalledOnce();
  expect(useCommandCenter.getState().chatDockOpen).toBe(true);
  expect(useCommandCenter.getState().chatSessionId).toBe('existing-conversation');
  expect(ensureSession).not.toHaveBeenCalled();
  expect(connectSession).not.toHaveBeenCalled();
  expect(sendMessage).not.toHaveBeenCalled();
});

it('retains existing history while disconnected and when opening shared chat', () => {
  const messages: ChatMessage[] = [{ id: 'message-1', role: 'assistant', content: 'Your saved reply', timestamp: '2026-10-07T12:00:00Z' }];
  useCommandCenter.setState({ chatMessages: messages });
  act(() => root.render(<ForumConversation />));
  expect(host.querySelector('article')?.textContent).toBe('Your saved reply');
  expect(host.textContent).not.toContain('No messages to show here yet.');
  act(() => button('Open shared chat').click());
  expect(useCommandCenter.getState().chatMessages).toBe(messages);
  expect(host.querySelector('article')?.textContent).toBe('Your saved reply');
  expect(sendMessage).not.toHaveBeenCalled();
});

it('preserves the active reply stop control and reports cancellation errors', async () => {
  useCommandCenter.setState({ isStreaming: true, connectionStatus: 'connected' });
  act(() => root.render(<ForumConversation />));
  expect(button('Stop reply').disabled).toBe(true);
  expect(host.querySelector('[role="status"]')?.textContent).toContain('Reply in progress');
  stopStreaming.mockRejectedValueOnce(new Error('Connection closed'));
  act(() => useCommandCenter.setState({ _activeRequestId: 'request-1' }));
  expect(button('Stop reply').disabled).toBe(false);
  await act(async () => button('Stop reply').click());
  expect(stopStreaming).toHaveBeenCalledOnce();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain('Could not stop the reply.');
});
