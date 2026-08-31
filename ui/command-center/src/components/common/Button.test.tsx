/**
 * @vitest-environment jsdom
 *
 * Button — the states a screenshot would show. Hover and :active live in CSS
 * and cannot be asserted here; what is asserted is that every state a caller
 * can drive from props is actually reachable, so those screenshots can be
 * produced from props alone.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Button, SUCCESS_FLASH_MS } from './Button';
import { getThemedColors } from '../../styles/tokens';

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

it('carries the variant class and theme colors as custom properties', () => {
  act(() => { root.render(<Button colors={colors} variant="primary">Go</Button>); });
  expect(btn().className).toContain('pa-btn');
  expect(btn().className).toContain('pa-btn--primary');
  expect(btn().style.getPropertyValue('--pa-btn-bg')).toBe(colors.cyan);
  expect(btn().style.getPropertyValue('--pa-btn-fg')).toBe(colors.textOnCyan);
});

it('shows a spinner while a promise-returning click is in flight', async () => {
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
  expect(btn().getAttribute('data-pending')).toBeNull();
  expect(btn().disabled).toBe(false);
});

it('ticks on success and drops the tick again', async () => {
  vi.useFakeTimers();
  act(() => {
    root.render(<Button colors={colors} onClick={() => Promise.resolve(true)}>Add</Button>);
  });
  await act(async () => { btn().click(); await Promise.resolve(); });
  expect(btn().getAttribute('data-state')).toBe('success');
  expect(container.querySelector('.pa-btn__tick')).toBeTruthy();

  act(() => { vi.advanceTimersByTime(SUCCESS_FLASH_MS + 10); });
  expect(btn().getAttribute('data-state')).toBeNull();
});

it('never ticks when the action reports failure', async () => {
  // `mutate` swallows its own errors and resolves `false` — the tick must not
  // congratulate the user for an update that did not land.
  act(() => {
    root.render(<Button colors={colors} onClick={() => Promise.resolve(false)}>Add</Button>);
  });
  await flush();
  await act(async () => { btn().click(); await Promise.resolve(); });
  expect(btn().getAttribute('data-state')).toBeNull();
  expect(btn().getAttribute('data-pending')).toBeNull();
});

it('never ticks when the promise rejects', async () => {
  act(() => {
    root.render(<Button colors={colors} onClick={() => Promise.reject(new Error('nope'))}>Add</Button>);
  });
  await act(async () => { btn().click(); await Promise.resolve(); });
  expect(btn().getAttribute('data-state')).toBeNull();
  expect(btn().getAttribute('data-pending')).toBeNull();
});

it('takes pending and success from props for form submits', () => {
  act(() => { root.render(<Button colors={colors} type="submit" pending>Save</Button>); });
  expect(btn().getAttribute('data-pending')).toBe('true');
  act(() => { root.render(<Button colors={colors} type="submit" success>Save</Button>); });
  expect(btn().getAttribute('data-state')).toBe('success');
});

it('stays disabled and unclicked when disabled', () => {
  const onClick = vi.fn();
  act(() => { root.render(<Button colors={colors} disabled onClick={onClick}>Nope</Button>); });
  expect(btn().disabled).toBe(true);
  act(() => { btn().click(); });
  expect(onClick).not.toHaveBeenCalled();
});
