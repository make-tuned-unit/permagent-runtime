/**
 * @vitest-environment jsdom
 *
 * FormModal — the modal you fill in.
 *
 * What is asserted here is the difference between this and the six hand-rolled
 * form modals it replaces: it is a real `<form>` so Enter submits, the submit
 * button takes the shell's own in-flight flag (the form-submit shape, where the
 * work is on `onSubmit` and the click cannot be awaited), a failure keeps the
 * modal open with a sentence on it instead of closing as though it worked, and
 * the a11y floor is INHERITED from `DetailModal` rather than re-implemented —
 * which is checked here by asserting the dialog semantics are present without
 * this file containing a line of code that produces them.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { FormModal, type FormModalProps } from './FormModal';
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
    b => (b.textContent ?? '').trim().replace(/^[✓]\s*/, '') === text,
  );
  expect(found, `no button labelled "${text}"`).toBeDefined();
  return found as HTMLButtonElement;
}

async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

function render(props: Partial<FormModalProps> = {}) {
  const full: FormModalProps = {
    title: 'New automation',
    submitLabel: 'Create',
    onSubmit: () => undefined,
    onCancel: () => {},
    children: <input name="name" defaultValue="Nightly sweep" />,
    ...props,
  };
  act(() => root.render(<FormModal {...full} />));
  return full;
}

it('inherits the dialog floor rather than re-implementing it', () => {
  render();
  const dialog = container.querySelector('[role="dialog"]');
  expect(dialog, 'no role=dialog — the shell is not composing DetailModal').not.toBeNull();
  expect(dialog?.getAttribute('aria-modal')).toBe('true');
  const labelId = dialog?.getAttribute('aria-labelledby');
  expect(labelId).toBeTruthy();
  // `getElementById`, not a `#id` selector: React's `useId` produces colons,
  // which are legal in an id and illegal in a bare CSS selector.
  expect(document.getElementById(labelId!)?.textContent).toBe('New automation');
});

it('is a real form, so Enter submits from a field', async () => {
  const onSubmit = vi.fn();
  render({ onSubmit });
  const form = container.querySelector('form');
  expect(form, 'the fields are not inside a <form>').not.toBeNull();
  await act(async () => {
    form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
  expect(onSubmit).toHaveBeenCalledTimes(1);
});

it('wires the footer submit to the form in the body', () => {
  render();
  const form = container.querySelector('form')!;
  expect(buttonLabelled('Create').getAttribute('form')).toBe(form.id);
  expect(buttonLabelled('Create').getAttribute('type')).toBe('submit');
});

it('shows the submit as busy for the whole round trip, floor included', async () => {
  let release: () => void = () => {};
  render({ onSubmit: () => new Promise<void>(r => { release = r; }) });
  const form = container.querySelector('form')!;
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
  expect(buttonLabelled('Create').getAttribute('aria-busy')).toBe('true');
  await act(async () => { release(); for (let i = 0; i < 8; i += 1) await Promise.resolve(); });
  // Still busy: the pending floor holds a fast submit visible.
  expect(buttonLabelled('Create').getAttribute('aria-busy')).toBe('true');
  await advance(MIN_PENDING_MS + 20);
  expect(buttonLabelled('Create').getAttribute('aria-busy')).toBeNull();
});

it('keeps itself open with the reason when the submit resolves false', async () => {
  const onCancel = vi.fn();
  render({ onSubmit: () => false, failureLabel: "Couldn't create it", onCancel });
  const form = container.querySelector('form')!;
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
  await advance(MIN_PENDING_MS + 20);
  expect(container.querySelector('[role="alert"]')?.textContent).toBe("Couldn't create it");
  expect(onCancel, 'a failed submit must not close the modal').not.toHaveBeenCalled();
});

it('puts the thrown reason in the sentence', async () => {
  render({
    onSubmit: () => { throw new Error('name already taken'); },
    failureLabel: "Couldn't create it",
  });
  const form = container.querySelector('form')!;
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
  await advance(MIN_PENDING_MS + 20);
  expect(container.querySelector('[role="alert"]')?.textContent)
    .toBe("Couldn't create it — name already taken");
});

it('clears a stale failure when the user tries again', async () => {
  let fail = true;
  render({ onSubmit: () => (fail ? false : undefined) });
  const form = container.querySelector('form')!;
  const submit = async () => {
    await act(async () => {
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      for (let i = 0; i < 8; i += 1) await Promise.resolve();
    });
    await advance(MIN_PENDING_MS + 20);
  };
  await submit();
  expect(container.querySelector('[role="alert"]')).not.toBeNull();
  fail = false;
  await submit();
  expect(container.querySelector('[role="alert"]')).toBeNull();
});

it('says why the submit is disabled instead of just dimming it', () => {
  render({ submitDisabled: true, disabledReason: 'Give the automation a name first.' });
  expect(buttonLabelled('Create').disabled).toBe(true);
  expect(buttonLabelled('Create').getAttribute('title')).toBe('Give the automation a name first.');
  expect(container.textContent).toContain('Give the automation a name first.');
});

it('makes Cancel a peer of the submit, and blocks it mid-flight', async () => {
  const onCancel = vi.fn();
  let release: () => void = () => {};
  render({ onCancel, onSubmit: () => new Promise<void>(r => { release = r; }) });
  expect(buttonLabelled('Cancel').disabled).toBe(false);
  const form = container.querySelector('form')!;
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
  expect(buttonLabelled('Cancel').disabled).toBe(true);
  await act(async () => { release(); for (let i = 0; i < 8; i += 1) await Promise.resolve(); });
  await advance(MIN_PENDING_MS + 20);
  await settle();
  act(() => { buttonLabelled('Cancel').dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  expect(onCancel).toHaveBeenCalled();
});
