/**
 * @vitest-environment jsdom
 *
 * DetailModal is the base every modal in the app is meant to sit on, so its
 * keyboard floor is the app's keyboard floor. It had Escape and a scrim click
 * and nothing else: no dialog semantics for assistive tech, no focus moved into
 * the panel on open, and Tab walked straight out to the page behind the scrim —
 * which is how a keyboard user ends up operating a screen they can't see.
 *
 * Fixed here rather than per consumer, so ConfirmDialog and every migrated
 * bespoke modal inherit it.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { DetailModal } from './DetailModal';

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
});

function dialog(): HTMLElement {
  // The modal portals to <body>, so look there, not inside the render root.
  const el = document.querySelector('[role="dialog"]');
  expect(el, 'the panel must be a dialog for assistive tech').not.toBeNull();
  return el as HTMLElement;
}

function press(key: string, init: KeyboardEventInit = {}) {
  act(() => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...init }));
  });
}

function render(children: React.ReactNode, onClose = vi.fn()) {
  act(() => {
    root.render(
      <DetailModal title="Nightly sweep" onClose={onClose} footer={<button>Save</button>}>
        {children}
      </DetailModal>,
    );
  });
  return onClose;
}

it('is a modal dialog, labelled by its own title', () => {
  render(<button>Inside</button>);
  const panel = dialog();
  expect(panel.getAttribute('aria-modal')).toBe('true');
  const labelledBy = panel.getAttribute('aria-labelledby');
  expect(labelledBy, 'the title must name the dialog').toBeTruthy();
  expect(document.getElementById(labelledBy!)?.textContent).toBe('Nightly sweep');
});

it('moves focus into the panel on open', () => {
  render(<button>Inside</button>);
  expect(dialog().contains(document.activeElement)).toBe(true);
});

it('coordinator finding: focuses the close button, not the bare panel div — a script-focused non-interactive element trips :focus-visible even from a pointer-opened modal', () => {
  render(<button>Inside</button>);
  const panel = dialog();
  // The close button is a real `<button>` (a normally-focusable element), so
  // it inherits the browser's actual input modality; the panel itself is a
  // `<div tabindex="-1">`, which Chromium always shows a focus-visible ring
  // on when focused by script, regardless of how the modal was opened.
  expect(document.activeElement?.tagName).toBe('BUTTON');
  expect(panel.contains(document.activeElement)).toBe(true);
  expect(panel).not.toBe(document.activeElement);
});

it('leaves an autofocused field alone', () => {
  render(<input autoFocus data-testid="field" />);
  expect((document.activeElement as HTMLElement)?.dataset?.testid).toBe('field');
});

it('keeps Tab inside the panel, in both directions', () => {
  render(<><button>First inside</button><button>Second inside</button></>);
  const panel = dialog();
  const focusables = Array.from(panel.querySelectorAll('button'));
  const first = focusables[0];
  const last = focusables[focusables.length - 1];

  act(() => { last.focus(); });
  press('Tab');
  expect(document.activeElement).toBe(first);

  act(() => { first.focus(); });
  press('Tab', { shiftKey: true });
  expect(document.activeElement).toBe(last);
});

it('pulls focus back when it has escaped the panel entirely', () => {
  const outside = document.createElement('button');
  document.body.appendChild(outside);
  render(<button>Inside</button>);

  act(() => { outside.focus(); });
  press('Tab');
  expect(dialog().contains(document.activeElement)).toBe(true);
  outside.remove();
});

it('restores focus to whatever opened it', () => {
  const opener = document.createElement('button');
  document.body.appendChild(opener);
  opener.focus();

  render(<button>Inside</button>);
  expect(document.activeElement).not.toBe(opener);

  act(() => { root.unmount(); });
  expect(document.activeElement).toBe(opener);

  opener.remove();
  root = createRoot(container);
});

it('still closes on Escape and on the scrim, and not on the panel itself', () => {
  const onClose = render(<button>Inside</button>);
  press('Escape');
  expect(onClose).toHaveBeenCalledTimes(1);

  act(() => {
    dialog().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
  expect(onClose, 'a click inside the panel must not close it').toHaveBeenCalledTimes(1);

  // The scrim is the dialog's parent, portaled to <body>.
  const scrim = dialog().parentElement as HTMLElement;
  act(() => { scrim.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  expect(onClose).toHaveBeenCalledTimes(2);
});

it('escapes a transformed ancestor: the overlay is portaled to <body>', () => {
  // A dashboard card carries a transform, which makes it the containing block
  // for `position: fixed` — a modal rendered in place was trapped in the card.
  // jsdom has no layout, so assert the structural guarantee instead: the
  // overlay is not inside the transformed ancestor at all.
  const card = document.createElement('div');
  card.style.transform = 'scale(1)';
  card.style.overflow = 'hidden';
  document.body.appendChild(card);
  const cardRoot = createRoot(card);
  act(() => {
    cardRoot.render(
      <DetailModal title="Decisions" onClose={vi.fn()}>
        <button>Inside</button>
      </DetailModal>,
    );
  });
  const panel = document.querySelector('[role="dialog"]') as HTMLElement;
  expect(panel).not.toBeNull();
  expect(card.contains(panel), 'the dialog must not live inside the card').toBe(false);
  const overlay = panel.parentElement as HTMLElement;
  expect(overlay.parentElement).toBe(document.body);
  expect(overlay.style.position).toBe('fixed');
  act(() => cardRoot.unmount());
  expect(document.querySelector('[role="dialog"]'), 'unmount removes the portal').toBeNull();
  card.remove();
});

// ── GM5/R3: 2-line title clamp, tooltip only when actually clamped ─────────
// jsdom has no layout, so `scrollHeight`/`clientHeight` are always 0/0 for
// every element — there is nothing to stub on the title node AFTER it
// mounts, since the measurement effect's first (synchronous) read happens
// during that same mount. Stubbing the two getters on `HTMLElement.prototype`
// BEFORE rendering makes every element in the test report the same box,
// which is enough to prove the branch: the mount-time read decides whether a
// Tooltip (and its tabIndex=-1 stop) is present at all.

function stubEveryElementBox(scrollHeight: number, clientHeight: number) {
  Object.defineProperty(HTMLElement.prototype, 'scrollHeight', { configurable: true, get: () => scrollHeight });
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get: () => clientHeight });
}

afterEach(() => {
  delete (HTMLElement.prototype as unknown as Record<string, unknown>).scrollHeight;
  delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientHeight;
});

it('gets no Tooltip when the title fits within its 2-line box', () => {
  stubEveryElementBox(20, 20); // scrollHeight === clientHeight: nothing is clipped
  render(<button>Inside</button>);
  const title = document.getElementById(dialog().getAttribute('aria-labelledby')!)!;
  expect(title.getAttribute('tabindex'), 'no Tooltip means no tabIndex stop either').toBeNull();
  expect(title.closest('[data-radix-tooltip], [role="tooltip"]')).toBeNull();
});

it('gets a Tooltip (and the tabIndex=-1 stop for it) once the title is actually clamped', () => {
  stubEveryElementBox(40, 20); // scrollHeight > clientHeight: the 2-line box is clipping text
  render(<button>Inside</button>);
  const title = document.getElementById(dialog().getAttribute('aria-labelledby')!)!;
  expect(title.getAttribute('tabindex')).toBe('-1');
});
