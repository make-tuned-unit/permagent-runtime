/** @vitest-environment jsdom
 *
 * Deleting an automation is Tier 2 by the destructive-action ruling —
 * destructive, but recoverable with effort: the recipe can be created again,
 * and the runs it already made stay in Recent Activity either way.
 *
 * That tier confirms INLINE, on the row, so what is about to go stays on
 * screen. It does not spend a full-attention modal (that is Tier 3, for the
 * unrecoverable — rotating a live drain key), and it certainly does not spend
 * an OS dialog. This file is the guard on that: the first click must ask, in
 * place, and no dialog may open.
 */

import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { DeleteAutomationControl } from './AutomateView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

function buttons(): HTMLButtonElement[] {
  return Array.from(container.querySelectorAll('button'));
}

function byLabel(text: string): HTMLButtonElement | undefined {
  return buttons().find(b => (b.textContent || '').trim() === text);
}

function click(el: Element) {
  act(() => {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
}

async function flush() {
  await act(async () => { await Promise.resolve(); });
}

function render(onDelete: () => Promise<void>) {
  act(() => {
    root.render(<DeleteAutomationControl name="Nightly sweep" onDelete={onDelete} />);
  });
}

describe('DeleteAutomationControl', () => {
  it('asks on the row instead of deleting, and never opens a dialog', () => {
    const onDelete = vi.fn(async () => {});
    render(onDelete);

    click(byLabel('Delete')!);

    expect(onDelete).not.toHaveBeenCalled();
    // The two-step, in place: an explicit affirmative next to a Cancel.
    expect(byLabel('Delete automation')).toBeTruthy();
    expect(byLabel('Cancel')).toBeTruthy();
    // What is being deleted is named where the user is looking.
    expect(container.textContent).toContain('Nightly sweep');
    // Tier 2 does not interrupt: no modal, and no OS dialog either.
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it('backs out cleanly', () => {
    const onDelete = vi.fn(async () => {});
    render(onDelete);

    click(byLabel('Delete')!);
    click(byLabel('Cancel')!);

    expect(onDelete).not.toHaveBeenCalled();
    expect(byLabel('Delete')).toBeTruthy();
    expect(byLabel('Delete automation')).toBeUndefined();
  });

  it('deletes once the affirmative is clicked', async () => {
    const onDelete = vi.fn(async () => {});
    render(onDelete);

    click(byLabel('Delete')!);
    click(byLabel('Delete automation')!);
    await flush();

    expect(onDelete).toHaveBeenCalledTimes(1);
  });

  it('states a failure on the control and stays open to retry', async () => {
    const onDelete = vi.fn(async () => { throw new Error('daemon said no'); });
    render(onDelete);

    click(byLabel('Delete')!);
    click(byLabel('Delete automation')!);
    await flush();

    const alert = container.querySelector('[role="alert"]');
    expect(alert).toBeTruthy();
    expect(alert!.textContent).toContain('daemon said no');
    // A failed delete must not read as a done one: the confirm step is still
    // on screen, so the user can try again without hunting for the row.
    expect(byLabel('Delete automation')).toBeTruthy();
  });
});
