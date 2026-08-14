/** @vitest-environment jsdom
 *
 * 2026-08-11 report: "I want a Copy function when my agent writes a dispatch for
 * me so I dont have to select it and then copy." These pin the affordance and,
 * more importantly, its honesty — a copy button that shows nothing when the copy
 * fails is indistinguishable from one that is broken.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/store', () => {
  const state = { agentName: 'Henry' };
  const useCommandCenter = Object.assign(
    (selector: (v: typeof state) => unknown) => selector(state),
    { getState: () => state },
  );
  return { useCommandCenter };
});
vi.mock('./MessageRenderer', () => ({ MessageRenderer: () => null }));
vi.mock('../awareness/CitationMarker', () => ({ CitationMarker: () => null }));

import { MessageBubble } from './MessageBubble';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
let writeText: ReturnType<typeof vi.fn>;

function setClipboard(value: unknown) {
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value });
}

const msg = (over: Record<string, unknown>) => ({
  id: 'm1',
  role: 'assistant' as const,
  content: 'body',
  timestamp: '2026-08-11T10:30:00.000Z',
  ...over,
});

beforeEach(() => {
  writeText = vi.fn().mockResolvedValue(undefined);
  setClipboard({ writeText });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  setClipboard(undefined);
});

function copyButton() {
  return container.querySelector('button[aria-label="Copy message"]') as HTMLButtonElement | null;
}

async function render(message: unknown) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  await act(async () => { root.render(<MessageBubble message={message as any} />); });
}

describe('copy control on an agent message', () => {
  it('copies the dispatch body without the fences the agent wrapped it in', async () => {
    await render(msg({ content: '```\nHi Sam — can you send over the invoice?\n```' }));
    await act(async () => { copyButton()!.click(); });
    expect(writeText).toHaveBeenCalledWith('Hi Sam — can you send over the invoice?');
  });

  it('copies the agent\'s own text, never the name/timestamp chrome around it', async () => {
    await render(msg({ content: 'Booked for Tuesday.' }));
    await act(async () => { copyButton()!.click(); });
    const copied = writeText.mock.calls[0][0] as string;
    expect(copied).toBe('Booked for Tuesday.');
    expect(copied).not.toContain('Henry');
    // The rendered bubble does carry the chrome — this is what was NOT copied.
    expect(container.textContent).toContain('Henry');
  });

  it('confirms the copy on screen and announces it to screen readers', async () => {
    await render(msg({}));
    expect(container.querySelector('[role="status"]')?.textContent).toBe('');
    await act(async () => { copyButton()!.click(); });
    expect(copyButton()!.textContent).toContain('Copied');
    expect(container.querySelector('[role="status"]')?.textContent).toBe('Message copied to clipboard');
  });

  it('says so when the copy fails instead of doing nothing visible', async () => {
    setClipboard(undefined); // insecure context, e.g. a paired device over LAN
    (document as unknown as Record<string, unknown>).execCommand = vi.fn().mockReturnValue(false);
    await render(msg({}));
    await act(async () => { copyButton()!.click(); });
    expect(copyButton()!.textContent).toContain('Copy failed');
    expect(container.querySelector('[role="status"]')?.textContent).toBe('Could not copy the message');
    delete (document as unknown as Record<string, unknown>).execCommand;
  });

  it('is a real button — reachable and operable from the keyboard, and labelled', async () => {
    await render(msg({}));
    const btn = copyButton()!;
    expect(btn.tagName).toBe('BUTTON');
    expect(btn.type).toBe('button');
    expect(btn.getAttribute('aria-label')).toBe('Copy message');
    expect(btn.hasAttribute('disabled')).toBe(false);
    // Enter/Space on a focused <button> dispatch a click; no tabIndex override
    // or aria-hidden may get in the way.
    expect(btn.getAttribute('tabindex')).toBeNull();
    expect(btn.closest('[aria-hidden="true"]')).toBeNull();
  });

  it('offers nothing to copy on the reader\'s own messages, or on an empty one', async () => {
    await render(msg({ role: 'user' }));
    expect(copyButton()).toBeNull();
    await render(msg({ content: '   ' }));
    expect(copyButton()).toBeNull();
  });
});
