/**
 * @vitest-environment jsdom
 *
 * ConfirmDialog — the Tier-3 destructive confirmation.
 *
 * What is asserted here is the difference between this and the native
 * `confirm()` it replaces: the dialog states the *specific consequence* rather
 * than asking "are you sure?", it runs the affirmative action through the
 * Button contract (pending, no tick on failure), and a failed action leaves the
 * dialog open with a sentence on it instead of closing as though it worked.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { ConfirmDialog } from './ConfirmDialog';
import { MIN_PENDING_MS } from './Button';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

function buttonLabelled(text: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll('button')).find(
    b => (b.textContent ?? '').trim() === text,
  );
  expect(found, `no button labelled "${text}"`).toBeDefined();
  return found as HTMLButtonElement;
}

async function click(el: HTMLButtonElement) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

function render(props: Partial<Parameters<typeof ConfirmDialog>[0]> = {}) {
  const merged = {
    title: 'Mint a new drain key?',
    consequence: 'Ingestion will fail with 401 until you redeploy.',
    confirmLabel: 'Mint a new key',
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
    ...props,
  };
  act(() => { root.render(<ConfirmDialog {...merged} />); });
  return merged;
}

it('states the consequence rather than asking "are you sure?"', () => {
  render();
  const text = container.textContent ?? '';
  expect(text).toContain('Mint a new drain key?');
  expect(text).toContain('Ingestion will fail with 401 until you redeploy.');
  expect(text.toLowerCase()).not.toContain('are you sure');
});

it('runs the affirmative action and leaves closing to the caller', async () => {
  const props = render();
  await click(buttonLabelled('Mint a new key'));
  expect(props.onConfirm).toHaveBeenCalledTimes(1);
  expect(props.onCancel).not.toHaveBeenCalled();
});

it('cancels without running the action', async () => {
  const props = render();
  await click(buttonLabelled('Cancel'));
  expect(props.onCancel).toHaveBeenCalledTimes(1);
  expect(props.onConfirm).not.toHaveBeenCalled();
});

it('Escape cancels — the shell it is built on owns that', () => {
  const props = render();
  act(() => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
  });
  expect(props.onCancel).toHaveBeenCalledTimes(1);
});

it('keeps the dialog open and names the failure when the action rejects', async () => {
  const props = render({
    onConfirm: vi.fn().mockRejectedValue(new Error('503 from the daemon')),
    failureLabel: "Couldn't mint a new key",
  });

  await click(buttonLabelled('Mint a new key'));
  await advance(MIN_PENDING_MS + 50);

  const alert = container.querySelector('[role="alert"]');
  expect(alert, 'a failed confirm must say so on the dialog').not.toBeNull();
  expect(alert!.textContent).toContain("Couldn't mint a new key");
  expect(alert!.textContent).toContain('503 from the daemon');
  // Still open, and the affirmative never ticked.
  expect(container.textContent).toContain('Mint a new drain key?');
  expect(buttonLabelled('Mint a new key').getAttribute('data-state')).not.toBe('success');
  expect(props.onCancel).not.toHaveBeenCalled();
});

it('treats a `false` resolution as a failure too — nothing ticks', async () => {
  render({ onConfirm: vi.fn().mockResolvedValue(false), failureLabel: "Couldn't remove it" });

  await click(buttonLabelled('Mint a new key'));
  await advance(MIN_PENDING_MS + 50);

  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Couldn't remove it");
  expect(buttonLabelled('Mint a new key').getAttribute('data-state')).not.toBe('success');
});
