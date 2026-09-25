/** @vitest-environment jsdom */
/**
 * QA at real size: a Recent row's title read `{name: 'scroll_depth', depth: ...`
 * — a session named after its tool-call args, cut at 40 chars by dashboard.rs.
 * The row must show a human label, in the text and the accessible name alike.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

vi.mock('../../../lib/store', () => ({
  useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({ switchToSession: () => Promise.resolve(), openChatDock: () => {} }),
}));

import { RecentCard } from './RecentCard';
import type { RecentSession } from '../useDashboard';

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

it('shows a human label, not a serialized args blob, as the row title', () => {
  const now = new Date().toISOString();
  const items: RecentSession[] = [
    { id: 's1', title: "{name: 'scroll_depth', depth: 0.75, sel...", state: 'completed', ended_at: now },
    { id: 's2', title: 'Plan the Atlas launch', state: 'completed', ended_at: now },
  ];
  act(() => root.render(<RecentCard items={items} />));
  const rows = Array.from(container.querySelectorAll('[role="button"]'));
  expect(rows).toHaveLength(2);
  expect(container.textContent).not.toContain('{name:');
  expect(rows[0].textContent).toContain('Scroll depth');
  expect(rows[0].getAttribute('aria-label')).toBe('Open conversation: Scroll depth');
  expect(rows[1].textContent).toContain('Plan the Atlas launch');
});

it('the title owns the row and shows its full text in the shared Tooltip; the status word gives way', async () => {
  const { _resetTooltipWarmForTests, TOOLTIP_COLD_DELAY_MS } = await import('../../common/Tooltip');
  const long = 'Fix event_cta_click funnel leak: 3,836 impressions, 0 clicks on the pricing page';
  vi.useFakeTimers();
  try {
    _resetTooltipWarmForTests();
    act(() => root.render(<RecentCard items={[{ id: 's1', title: long, state: 'awaiting_input', ended_at: new Date().toISOString() }]} />));
    const title = container.querySelector('[data-testid="recent-title"]') as HTMLElement;
    const status = container.querySelector('[data-testid="recent-status"]') as HTMLElement;
    expect(title.textContent).toBe(long);
    expect(title.hasAttribute('title')).toBe(false);
    // Before: the status was `flexShrink: 0`, so it never yielded width.
    expect(status.style.flexShrink).not.toBe('0');
    expect(status.style.minWidth).toMatch(/^0(px)?$/);
    expect((title.parentElement as HTMLElement).style.flexGrow).toBe('1');
    act(() => { title.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
    act(() => { vi.advanceTimersByTime(TOOLTIP_COLD_DELAY_MS); });
    expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(long);
    act(() => { title.dispatchEvent(new MouseEvent('mouseout', { bubbles: true })); });
  } finally {
    vi.useRealTimers();
  }
});
