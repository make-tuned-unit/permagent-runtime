/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const fetcher = vi.hoisted(() => vi.fn());
vi.mock('../../lib/api', () => ({ apiFetch: fetcher }));
import { DEFAULT_LAYOUT, useLayout } from './useLayout';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement; let resolve: (value: unknown) => void; let reject: (reason: unknown) => void;
let shownLayout = DEFAULT_LAYOUT;
function Probe() { const { layoutLoaded, layout } = useLayout(); shownLayout = layout; return <span>{layoutLoaded ? 'ready' : 'loading'}</span>; }
beforeEach(() => {
  localStorage.clear();
  fetcher.mockReset();
  fetcher.mockImplementationOnce(() => new Promise((yes, no) => { resolve = yes; reject = no; }));
  fetcher.mockResolvedValue(DEFAULT_LAYOUT);
  host = document.createElement('div'); root = createRoot(host);
  act(() => root.render(<Probe />));
});
afterEach(() => { act(() => root.unmount()); });
it('does not expose the initial default layout as loaded while the saved layout is pending', async () => {
  expect(host.textContent).toBe('loading');
  await act(async () => { resolve(DEFAULT_LAYOUT); });
  expect(host.textContent).toBe('ready');
});
it('marks the displayed fallback ready when the saved layout fails', async () => {
  expect(host.textContent).toBe('loading');
  await act(async () => { reject(new Error('offline')); });
  expect(host.textContent).toBe('ready');
});
it('does not auto-add old cards or write a loaded daily default', async () => {
  await act(async () => { resolve(DEFAULT_LAYOUT); });
  expect(fetcher).toHaveBeenCalledTimes(1);
  expect(fetcher.mock.calls[0][0]).toBe('/api/dashboard/layout');
});

it('preserves custom sizes and omitted cards on a fresh device without writing', async () => {
  const custom = { cards: [{ ...DEFAULT_LAYOUT.cards[2], size: { w: 4, h: 7 } }] };
  await act(async () => { resolve(custom); });
  expect(shownLayout).toEqual(custom);
  expect(fetcher).toHaveBeenCalledTimes(1);
});
