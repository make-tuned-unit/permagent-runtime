/** @vitest-environment jsdom */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { createRef } from 'react';

vi.mock('../voice/VoiceButton', () => ({ VoiceButton: () => null }));

import { ChatInput, type ChatInputHandle } from './ChatInput';
import { useCommandCenter } from '../../lib/store';
import { peekChatSetupIntent, stashChatSetupIntent, stashWizardIntent } from '../../lib/wizardIntent';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
let originalSendMessage: ReturnType<typeof useCommandCenter.getState>['sendMessage'];
let originalChatWindowOpen: boolean;
const field = () => container.querySelector('textarea') as HTMLTextAreaElement;

function type(text: string) {
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    setter.call(field(), text);
    field().dispatchEvent(new Event('input', { bubbles: true }));
  });
}

beforeEach(() => {
  localStorage.clear();
  window.history.replaceState({}, '', '/');
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  originalSendMessage = useCommandCenter.getState().sendMessage;
  originalChatWindowOpen = useCommandCenter.getState().chatWindowOpen;
  act(() => useCommandCenter.setState({ isStreaming: false, chatDraft: { text: '', files: [] } }));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.setState({ sendMessage: originalSendMessage, chatWindowOpen: originalChatWindowOpen });
  localStorage.clear();
  window.history.replaceState({}, '', '/');
});

it('restores a typed and attached draft after the dock composer unmounts', () => {
  const first = createRef<ChatInputHandle>();
  act(() => root.render(<ChatInput ref={first} />));
  type('Keep this review draft');
  act(() => first.current?.addFiles([new File(['note'], 'review.txt', { type: 'text/plain' })]));
  act(() => root.unmount());
  root = createRoot(container);
  act(() => root.render(<ChatInput />));
  expect(field().value).toBe('Keep this review draft');
  expect(container.textContent).toContain('review.txt');
});

it('waits for a detached draft handoff before showing a queued setup request', () => {
  window.history.replaceState({}, '', '/?view=chat&draft_handoff=test-marker');
  const ref = createRef<ChatInputHandle>();
  act(() => root.render(<ChatInput ref={ref} />));
  expect(field().value).toBe('');
  expect(container.textContent).toContain('Restoring your unsent draft');
  expect(peekChatSetupIntent()).toBeNull();

  act(() => ref.current?.restoreDraft('Keep my draft', [new File(['note'], 'review.txt')], {
    v: 1, id: 'setup-from-main', text: 'Help me set up Tailscale', issuedAt: Date.now(),
  }));
  expect(field().value).toBe('Keep my draft');
  expect(container.textContent).toContain('review.txt');
  expect(container.textContent).toContain('Tailscale setup request is ready');
  expect(peekChatSetupIntent()).toBe('Help me set up Tailscale');
});

it('keeps a new setup request pending while an empty detached draft is frozen for return', () => {
  window.history.replaceState({}, '', '/?view=chat');
  const ref = createRef<ChatInputHandle>();
  act(() => root.render(<ChatInput ref={ref} />));
  let snapshot: ReturnType<ChatInputHandle['freezeDraft']> = null;
  act(() => { snapshot = ref.current?.freezeDraft() ?? null; });
  expect(snapshot).toMatchObject({ text: '', files: [] });
  act(() => { expect(stashChatSetupIntent('Help me set up Tailscale')).toBe(true); });
  expect(field().value).toBe('');
  expect(peekChatSetupIntent()).toBe('Help me set up Tailscale');
  act(() => ref.current?.releaseDraft());
  expect(field().value).toBe('Help me set up Tailscale');
});

it('shows a stored setup request for editing when Chat opens, without sending it', () => {
  expect(stashChatSetupIntent('Help me set up Tailscale')).toBe(true);
  act(() => root.render(<ChatInput />));
  expect(field().value).toBe('Help me set up Tailscale');
  expect(peekChatSetupIntent()).toBeNull();
});

it('hands off to an already mounted composer without overwriting its draft', () => {
  act(() => root.render(<ChatInput />));
  type('My unsent message');
  act(() => { expect(stashChatSetupIntent('Help me set up Tailscale')).toBe(true); });
  expect(field().value).toBe('My unsent message');
  expect(container.textContent).toContain('Tailscale setup request is ready');
  expect(peekChatSetupIntent()).toBe('Help me set up Tailscale');

  type('');
  expect(field().value).toBe('Help me set up Tailscale');
  expect(peekChatSetupIntent()).toBeNull();
});

it('keeps a file-only draft separate until its attachment is removed', () => {
  const ref = createRef<ChatInputHandle>();
  act(() => root.render(<ChatInput ref={ref} />));
  act(() => ref.current?.addFiles([new File(['contents'], 'notes.txt', { type: 'text/plain' })]));
  act(() => { expect(stashChatSetupIntent('Help me set up Tailscale')).toBe(true); });
  expect(field().value).toBe('');
  expect(container.textContent).toContain('Tailscale setup request is ready');
  expect(peekChatSetupIntent()).toBe('Help me set up Tailscale');
  act(() => (container.querySelector('[aria-label="Remove notes.txt"]') as HTMLButtonElement).click());
  expect(field().value).toBe('Help me set up Tailscale');
});

it('can dismiss a queued setup request without changing the current draft', () => {
  act(() => root.render(<ChatInput />));
  type('My unsent message');
  act(() => { expect(stashChatSetupIntent('Help me set up Tailscale')).toBe(true); });
  act(() => (container.querySelector('summary') as HTMLElement).click());
  act(() => (Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Dismiss setup request') as HTMLButtonElement).click());
  expect(field().value).toBe('My unsent message');
  expect(peekChatSetupIntent()).toBeNull();
});

it('does not send or clear a draft when setup is known to need a model', () => {
  const sendMessage = vi.fn().mockResolvedValue(undefined);
  useCommandCenter.setState({ sendMessage });
  act(() => root.render(<ChatInput modelReadiness="needs-model" />));
  type('My setup request');
  act(() => (container.querySelector('[aria-label="Send message"]') as HTMLButtonElement).click());
  expect(sendMessage).not.toHaveBeenCalled();
  expect(field().value).toBe('My setup request');
  expect(container.querySelector('[role="alert"]')?.textContent).toContain('No model is configured');
});

it('does not consume the wizard intent in a StrictMode state initializer', () => {
  stashWizardIntent('My first message');
  act(() => root.render(<ChatInput />));
  expect(field().value).toBe('My first message');
  expect(localStorage.getItem('permagent-wizard-intent')).toBeNull();
});

it('leaves the request for the detached window while the main dock is losing ownership', () => {
  useCommandCenter.setState({ chatWindowOpen: true });
  expect(stashChatSetupIntent('Help me set up Tailscale')).toBe(true);
  act(() => root.render(<ChatInput />));
  expect(field().value).toBe('');
  expect(peekChatSetupIntent()).toBe('Help me set up Tailscale');

  act(() => useCommandCenter.setState({ chatWindowOpen: false }));
  expect(field().value).toBe('Help me set up Tailscale');
  expect(peekChatSetupIntent()).toBeNull();
});
