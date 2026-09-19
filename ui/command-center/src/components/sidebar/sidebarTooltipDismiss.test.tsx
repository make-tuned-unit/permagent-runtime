/**
 * @vitest-environment jsdom
 *
 * QA 2026-09-19, real window: after pressing a sidebar row the row keeps focus,
 * and its focus-opened tooltip ("Build ⌘4") stayed on screen over the World
 * view it had just switched to. Activating a row now closes its tooltip.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Sidebar } from './Sidebar';
import { COLD_DELAY_MS } from './SidebarTooltip';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.useFakeTimers();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => root.render(<Sidebar />));
});
afterEach(() => { act(() => root.unmount()); container.remove(); vi.useRealTimers(); });

const tooltip = () => document.getElementById('pa-sidebar-tooltip');

describe('sidebar row tooltip', () => {
  it('opens on focus and closes when the row is activated', () => {
    const row = Array.from(container.querySelectorAll('button'))
      .find(b => (b.getAttribute('aria-label') ?? '').startsWith('History')) as HTMLButtonElement;
    expect(row).toBeTruthy();
    // jsdom has no layout, so give the anchor a real rect for the placement.
    row.getBoundingClientRect = () => ({ x: 10, y: 200, left: 10, top: 200, right: 170, bottom: 236, width: 160, height: 36, toJSON: () => ({}) }) as DOMRect;
    act(() => row.focus());
    act(() => { vi.advanceTimersByTime(COLD_DELAY_MS + 10); });
    expect(tooltip(), 'focus should open the tooltip').not.toBeNull();
    act(() => row.click());
    expect(tooltip(), 'activation should close it').toBeNull();
  });
});
