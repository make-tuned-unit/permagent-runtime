/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import type { useDecisionDiscussion } from '../../../lib/decisionDiscussion';

const { useDiscussion } = vi.hoisted(() => ({ useDiscussion: vi.fn() }));
vi.mock('../../../lib/decisionDiscussion', () => ({ useDecisionDiscussion: useDiscussion }));
import { DiscussionPane } from './DiscussionPane';

let root: Root;
let container: HTMLDivElement;
let discussion: ReturnType<typeof useDecisionDiscussion>;
const navigated = vi.fn();
function render() {
  act(() => root.render(
    <div onKeyDown={navigated}>
      <DiscussionPane decisionId="decision" headline="<b>Ship it?</b>" agentName="Henry" />
    </div>,
  ));
}
function key(keyName: string, options: KeyboardEventInit = {}) {
  const event = new KeyboardEvent('keydown', { key: keyName, bubbles: true, cancelable: true, ...options });
  act(() => { container.querySelector('textarea')!.dispatchEvent(event); });
  return event;
}
function sendButton() {
  return container.querySelector('button[aria-label="Send message about this decision"]') as HTMLButtonElement | null;
}
beforeEach(() => {
  (globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
  navigated.mockClear();
  discussion = {
    status: 'idle', error: null, messages: [], draft: 'Question', sessionId: null,
    setDraft: vi.fn(), send: vi.fn(async () => {}), retry: vi.fn(async () => {}),
  };
  useDiscussion.mockImplementation(() => discussion);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  render();
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

describe('DiscussionPane', () => {
  it('keydown does not propagate navigation keys while editing (rulings 6, 11)', () => {
    for (const name of ['j', 'k', 'J', 'K', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown']) {
      expect(key(name).defaultPrevented).toBe(false);
    }
    expect(navigated).not.toHaveBeenCalled();
    key('Escape');
    expect(navigated).toHaveBeenCalledTimes(1);
  });

  it('j on Send bubbles out while j in the textarea stays contained (ruling 11)', () => {
    key('j');
    expect(navigated).not.toHaveBeenCalled();
    act(() => sendButton()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'j', bubbles: true })));
    expect(navigated).toHaveBeenCalledTimes(1);
  });

  it('Enter sends; Shift+Enter and composition preserve editing (ruling 11)', () => {
    expect(key('Enter', { shiftKey: true }).defaultPrevented).toBe(false);
    expect(key('Enter', { isComposing: true }).defaultPrevented).toBe(false);
    expect(discussion.send).not.toHaveBeenCalled();
    expect(key('Enter').defaultPrevented).toBe(true);
    expect(discussion.send).toHaveBeenCalledTimes(1);
    expect(navigated).not.toHaveBeenCalled();
  });

  it('is an inline composer: placeholder only when idle, a labelled input, messages as plain text (U7; rulings 6, 7)', () => {
    const input = container.querySelector('textarea')!;
    expect(input.placeholder).toBe('Ask Henry about this…');
    expect(container.querySelector('[role="log"]')).toBeNull();
    expect(container.querySelector('h3')).toBeNull();
    expect(container.querySelector('label')!.htmlFor).toBe(input.id);
    discussion.messages = [
      { id: 'user', role: 'user', text: '<b>Question</b>' },
      { id: 'assistant', role: 'assistant', text: '<script>answer</script>', streaming: true },
    ];
    render();
    expect(container.textContent).toContain('<b>Question</b>');
    expect(container.textContent).toContain('<script>answer</script>');
    expect(container.querySelector('b, script')).toBeNull();
    expect(container.querySelector('[aria-label="Assistant is responding"]')).not.toBeNull();
    expect(container.querySelector('[role="log"]')!.textContent).toContain('Henry');
    expect(container.querySelector('[title]')).toBeNull();
  });

  it.each(['creating', 'sending'] as const)('disables input and Send while %s (ruling 6)', status => {
    discussion.status = status;
    render();
    expect(container.querySelector('textarea')!.disabled).toBe(true);
    expect(sendButton()!.disabled).toBe(true);
    key('Enter');
    expect(discussion.send).not.toHaveBeenCalled();
  });

  it('shows the daemon error and retries without discarding the input (ruling 7)', () => {
    discussion.status = 'error';
    discussion.error = 'daemon is offline';
    render();
    expect(container.querySelector('[role="alert"]')!.textContent).toContain('daemon is offline');
    expect(container.querySelector('textarea')!.value).toBe('Question');
    const retry = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Retry')!;
    act(() => retry.click());
    expect(discussion.retry).toHaveBeenCalledTimes(1);
    expect(discussion.setDraft).not.toHaveBeenCalled();
  });
});
