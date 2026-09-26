/**
 * @vitest-environment jsdom
 *
 * Button — the states a screenshot would show. Hover and :active live in CSS
 * and cannot be asserted here; what is asserted is that every state a caller
 * can drive from props is actually reachable, so those screenshots can be
 * produced from props alone.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Button, FAILURE_FLASH_MS, MIN_PENDING_MS, SUCCESS_FLASH_MS } from './Button';
import { getThemedColors } from '../../styles/tokens';
import { useNotifications, type AppNotification } from '../../lib/notifications';

/** Reads the live notification list into a plain array a test can assert on,
 *  without pulling in a testing-library this repo doesn't otherwise use. */
function NotificationsProbe({ onItems }: { onItems: (items: AppNotification[]) => void }) {
  const { items } = useNotifications();
  onItems(items);
  return null;
}

const colors = getThemedColors();

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
  vi.useRealTimers();
});

function btn(): HTMLButtonElement {
  return container.querySelector('button') as HTMLButtonElement;
}

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

/** Move fake time forward and let the promise chain behind the pending floor
 *  settle — `Promise.all` needs several microtask turns, not one. */
async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}

it('carries the variant class and theme colors as custom properties', () => {
  act(() => { root.render(<Button colors={colors} variant="primary">Go</Button>); });
  expect(btn().className).toContain('pa-btn');
  expect(btn().className).toContain('pa-btn--primary');
  expect(btn().style.getPropertyValue('--pa-btn-bg')).toBe(colors.cyan);
  expect(btn().style.getPropertyValue('--pa-btn-fg')).toBe(colors.textOnCyan);
});

it('shows a spinner while a promise-returning click is in flight', async () => {
  vi.useFakeTimers();
  let release: (v: unknown) => void = () => {};
  const onClick = () => new Promise((res) => { release = res; });
  act(() => { root.render(<Button colors={colors} onClick={onClick}>Run scan</Button>); });

  expect(btn().getAttribute('data-pending')).toBeNull();
  act(() => { btn().click(); });
  expect(btn().getAttribute('data-pending')).toBe('true');
  expect(btn().getAttribute('aria-busy')).toBe('true');
  expect(btn().disabled).toBe(true);
  expect(container.querySelector('.pa-btn__spinner')).toBeTruthy();

  await act(async () => { release(undefined); await Promise.resolve(); });
  await advance(MIN_PENDING_MS + 10);
  expect(btn().getAttribute('data-pending')).toBeNull();
  expect(btn().disabled).toBe(false);
});

it('holds the pending phase for 700ms when the round trip is faster', async () => {
  // A save that lands in 20ms still has to *read* as a save. Without the floor
  // the spinner appears and vanishes inside one frame, which is
  // indistinguishable from the click having done nothing at all.
  vi.useFakeTimers();
  act(() => {
    root.render(<Button colors={colors} onClick={() => Promise.resolve(true)}>Save</Button>);
  });
  await act(async () => { btn().click(); await Promise.resolve(); });

  expect(btn().getAttribute('data-pending')).toBe('true');
  await advance(MIN_PENDING_MS - 50);
  expect(btn().getAttribute('data-pending')).toBe('true');
  expect(btn().getAttribute('data-state')).toBeNull();

  await advance(60);
  expect(btn().getAttribute('data-pending')).toBeNull();
  expect(btn().getAttribute('data-state')).toBe('success');
});

it('holds the floor on failure too, then shows the failure state (not a tick)', async () => {
  vi.useFakeTimers();
  act(() => {
    root.render(<Button colors={colors} onClick={() => Promise.reject(new Error('nope'))}>Save</Button>);
  });
  await act(async () => { btn().click(); await Promise.resolve(); });
  expect(btn().getAttribute('data-pending')).toBe('true');

  await advance(MIN_PENDING_MS - 50);
  expect(btn().getAttribute('data-pending')).toBe('true');
  await advance(60);
  expect(btn().getAttribute('data-pending')).toBeNull();
  // R8: a rejected promise is a genuine, uncaught failure — it now surfaces
  // as `data-state="failure"` (the CSS shake/tint), never as `"success"`.
  expect(btn().getAttribute('data-state')).toBe('failure');
});

it('holds the floor for prop-driven pending as well', async () => {
  // Form submits do their work in `onSubmit` and pass `pending` in; they get
  // the same floor, or the migration would leave them flickering.
  vi.useFakeTimers();
  act(() => { root.render(<Button colors={colors} type="submit" pending>Save</Button>); });
  expect(btn().getAttribute('data-pending')).toBe('true');

  act(() => { root.render(<Button colors={colors} type="submit">Save</Button>); });
  expect(btn().getAttribute('data-pending')).toBe('true');

  await advance(MIN_PENDING_MS + 10);
  expect(btn().getAttribute('data-pending')).toBeNull();
});

it('minPendingMs={0} opts out of the floor', async () => {
  vi.useFakeTimers();
  act(() => {
    root.render(
      <Button colors={colors} minPendingMs={0} onClick={() => Promise.resolve(true)}>Add</Button>,
    );
  });
  await act(async () => {
    btn().click();
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
  expect(btn().getAttribute('data-pending')).toBeNull();
  expect(btn().getAttribute('data-state')).toBe('success');
});

it('ticks on success and drops the tick again', async () => {
  vi.useFakeTimers();
  act(() => {
    root.render(<Button colors={colors} onClick={() => Promise.resolve(true)}>Add</Button>);
  });
  await act(async () => { btn().click(); await Promise.resolve(); });
  await advance(MIN_PENDING_MS + 10);
  expect(btn().getAttribute('data-state')).toBe('success');
  expect(container.querySelector('.pa-btn__tick')).toBeTruthy();

  act(() => { vi.advanceTimersByTime(SUCCESS_FLASH_MS + 10); });
  expect(btn().getAttribute('data-state')).toBeNull();
});

it('never ticks when the action reports failure', async () => {
  // `mutate` swallows its own errors and resolves `false` — the tick must not
  // congratulate the user for an update that did not land.
  vi.useFakeTimers();
  act(() => {
    root.render(<Button colors={colors} onClick={() => Promise.resolve(false)}>Add</Button>);
  });
  await flush();
  await act(async () => { btn().click(); await Promise.resolve(); });
  await advance(MIN_PENDING_MS + 10);
  expect(btn().getAttribute('data-state')).toBeNull();
  expect(btn().getAttribute('data-pending')).toBeNull();
});

it('never ticks when the promise rejects — it shows failure instead', async () => {
  vi.useFakeTimers();
  act(() => {
    root.render(<Button colors={colors} onClick={() => Promise.reject(new Error('nope'))}>Add</Button>);
  });
  await act(async () => { btn().click(); await Promise.resolve(); });
  await advance(MIN_PENDING_MS + 10);
  expect(btn().getAttribute('data-state')).toBe('failure');
  expect(btn().getAttribute('data-pending')).toBeNull();

  await advance(FAILURE_FLASH_MS + 10);
  expect(btn().getAttribute('data-state')).toBeNull();
});

it('R8: a rejected promise pushes a toast with the error message by default', async () => {
  vi.useFakeTimers();
  let items: AppNotification[] = [];
  act(() => { root.render(<NotificationsProbe onItems={(i) => { items = i; }} />); });
  act(() => {
    root.render(
      <>
        <NotificationsProbe onItems={(i) => { items = i; }} />
        <Button colors={colors} onClick={() => Promise.reject(new Error('network down'))}>Add</Button>
      </>,
    );
  });
  await act(async () => { btn().click(); await Promise.resolve(); });
  await advance(MIN_PENDING_MS + 10);

  expect(items[0]?.title).toBe('Action failed');
  expect(items[0]?.body).toBe('network down');
  expect(items[0]?.tone).toBe('danger');
});

it('R8/coordinator 2026-09-25: the failure toast bypasses the "System messages" preference — it is the direct consequence of the user\'s own click, not ambient chatter', async () => {
  vi.useFakeTimers();
  const { setNotificationPref } = await import('../../lib/notifications');
  setNotificationPref('system', false);
  let items: AppNotification[] = [];
  act(() => {
    root.render(
      <>
        <NotificationsProbe onItems={(i) => { items = i; }} />
        <Button colors={colors} onClick={() => Promise.reject(new Error('daemon unreachable'))}>Add</Button>
      </>,
    );
  });
  await act(async () => { btn().click(); await Promise.resolve(); });
  await advance(MIN_PENDING_MS + 10);

  expect(items[0]?.title).toBe('Action failed');
  expect(items[0]?.body).toBe('daemon unreachable');
  setNotificationPref('system', true);
});

it('a fast success right after a failure does not leave the button stuck showing failure', async () => {
  // Two independent flags (selfSuccess/selfFailure) shared one reset timer
  // ref — a quick retry that lands the opposite outcome before the first
  // flash's timer fired used to cancel that timer without ever clearing the
  // flag it owned, orphaning it at `true` until it resurfaced later.
  vi.useFakeTimers();
  let calls = 0;
  const onClick = () => (calls++ === 0 ? Promise.reject(new Error('nope')) : Promise.resolve(true));
  act(() => {
    root.render(<Button colors={colors} minPendingMs={0} onClick={onClick}>Save</Button>);
  });

  await act(async () => { btn().click(); for (let i = 0; i < 6; i += 1) await Promise.resolve(); });
  expect(btn().getAttribute('data-state')).toBe('failure');

  // Retry succeeds well within FAILURE_FLASH_MS of the first outcome.
  await act(async () => {
    vi.advanceTimersByTime(50);
    btn().click();
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
  expect(btn().getAttribute('data-state')).toBe('success');

  // Once the success flash itself ends, the button must not revert to the
  // earlier (orphaned) failure state.
  await advance(SUCCESS_FLASH_MS + 10);
  expect(btn().getAttribute('data-state')).toBeNull();
});

it('a fast failure right after a success does not leave the button stuck showing success', async () => {
  // The symmetric case: a success's reset timer gets cancelled by a
  // following failure before it fires, orphaning `selfSuccess` at `true`.
  vi.useFakeTimers();
  let calls = 0;
  const onClick = () => (calls++ === 0 ? Promise.resolve(true) : Promise.reject(new Error('nope')));
  act(() => {
    root.render(<Button colors={colors} minPendingMs={0} onClick={onClick}>Save</Button>);
  });

  await act(async () => { btn().click(); for (let i = 0; i < 6; i += 1) await Promise.resolve(); });
  expect(btn().getAttribute('data-state')).toBe('success');

  // Retry fails well within SUCCESS_FLASH_MS of the first outcome.
  await act(async () => {
    vi.advanceTimersByTime(50);
    btn().click();
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
  expect(btn().getAttribute('data-state')).toBe('failure');

  await advance(FAILURE_FLASH_MS + 10);
  expect(btn().getAttribute('data-state')).toBeNull();
});

it('R8: onErrorFeedback={false} suppresses the shake and the toast', async () => {
  vi.useFakeTimers();
  let items: AppNotification[] = [];
  act(() => {
    root.render(
      <>
        <NotificationsProbe onItems={(i) => { items = i; }} />
        <Button colors={colors} onErrorFeedback={false} onClick={() => Promise.reject(new Error('nope'))}>
          Add
        </Button>
      </>,
    );
  });
  const before = items.length;
  await act(async () => { btn().click(); await Promise.resolve(); });
  await advance(MIN_PENDING_MS + 10);
  expect(btn().getAttribute('data-state')).toBeNull();
  expect(items.length).toBe(before);
});

it('R8: resolving false still shows no failure state and no toast (caller already handled it)', async () => {
  vi.useFakeTimers();
  let items: AppNotification[] = [];
  act(() => {
    root.render(
      <>
        <NotificationsProbe onItems={(i) => { items = i; }} />
        <Button colors={colors} onClick={() => Promise.resolve(false)}>Add</Button>
      </>,
    );
  });
  const before = items.length;
  await act(async () => { btn().click(); await Promise.resolve(); });
  await advance(MIN_PENDING_MS + 10);
  expect(btn().getAttribute('data-state')).toBeNull();
  expect(items.length).toBe(before);
});

it('takes pending and success from props for form submits', async () => {
  vi.useFakeTimers();
  act(() => { root.render(<Button colors={colors} type="submit" pending>Save</Button>); });
  expect(btn().getAttribute('data-pending')).toBe('true');
  act(() => { root.render(<Button colors={colors} type="submit" success>Save</Button>); });
  await advance(MIN_PENDING_MS + 10);
  expect(btn().getAttribute('data-state')).toBe('success');
});

it('stays disabled and unclicked when disabled', () => {
  const onClick = vi.fn();
  act(() => { root.render(<Button colors={colors} disabled onClick={onClick}>Nope</Button>); });
  expect(btn().disabled).toBe(true);
  act(() => { btn().click(); });
  expect(onClick).not.toHaveBeenCalled();
});

/**
 * The hover half of the contract lives in CSS, so what is asserted here is the
 * shape of the rule rather than a computed color: jsdom does not apply the
 * stylesheet. It matters because the buttons being migrated onto this
 * primitive overwhelmingly brighten their *text* on hover, not their fill —
 * an icon button that goes muted → full is the single most common resting
 * pattern in the app. Without a hover-color variable those migrations would
 * each have to keep a pair of JS mouse handlers, which is the thing the
 * primitive exists to delete.
 */
it('lets a caller name the hover color, not just the hover fill', () => {
  const css = readFileSync(resolve(process.cwd(), 'src/index.css'), 'utf8');
  const hover = /\.pa-btn:hover:not\(:disabled\) \{([^}]*)\}/.exec(css)?.[1] ?? '';
  expect(hover).toContain('--pa-btn-fg-hover');
  // Defaulted to the resting color, so every existing button is untouched.
  expect(hover).toContain('var(--pa-btn-fg-hover, var(--pa-btn-fg');
});

/**
 * The label span is a seam for the spinner and the tick, not a layout. Tailwind's
 * reset makes every `svg` a block, and a block inside an inline span breaks the
 * line — so an icon-and-word button rendered its icon ABOVE its word. Four
 * separate migrations hit this and each worked around it differently, which is
 * the signal that it belongs at the base.
 */
it('does not let its label span become a layout of its own', () => {
  const css = readFileSync(resolve(process.cwd(), 'src/index.css'), 'utf8');
  const label = /\.pa-btn__label \{([^}]*)\}/.exec(css)?.[1] ?? '';
  expect(label).toContain('display: contents');
});

it('forwards a ref, so a caller can measure it', () => {
  const seen: (HTMLButtonElement | null)[] = [];
  act(() => { root.render(<Button colors={colors} ref={el => seen.push(el)}>Go</Button>); });
  expect(seen[0]).toBeInstanceOf(HTMLButtonElement);
});
