/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { DecisionsResponse } from '../decisions/types';

const fixture = vi.hoisted(() => ({
  data: null as DecisionsResponse | null, error: false, refresh: vi.fn(), pending: false, clear: vi.fn(),
}));
vi.mock('../decisions/useDecisions', () => ({ useDecisions: () => ({ ...fixture, loading: !fixture.data }) }));
vi.mock('../decisions/DecisionInbox', () => ({ DecisionInbox: ({ onClose }: { onClose: () => void }) => <div role="dialog"><button onClick={onClose}>Close</button></div> }));
vi.mock('../../../lib/store', () => ({ useCommandCenter: (select: (s: Record<string, unknown>) => unknown) => select({
  pendingDecisionInbox: fixture.pending, clearPendingDecisionInbox: fixture.clear,
}) }));
import { DecisionsCard } from './DecisionsCard';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement; let root: Root;
beforeEach(() => {
  fixture.data = { total_pending: 3, handled_count: 7, goals_needing_attention: 2,
    goals_in_flight: 4, oldest_pending_at: new Date(Date.now() - 3600_000).toISOString(),
    decisions: [{ id: 'one', headline: 'Review the launch plan' }], attention_goals: [],
  } as unknown as DecisionsResponse;
  fixture.error = false; fixture.pending = false; fixture.refresh.mockClear(); fixture.clear.mockClear();
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); });
function render() { act(() => root.render(<DecisionsCard />)); }

it('combines one queue count and a concrete request with one native review button', () => {
  render();
  expect(host.textContent).toContain('3 waiting');
  expect(host.textContent).toContain('Review the launch plan');
  expect(host.textContent).toContain('2 goals need your attention');
  expect(host.textContent).not.toContain('handled 7');
  expect(host.querySelectorAll('button')).toHaveLength(1);
  const button = host.querySelector<HTMLButtonElement>('[aria-label="Open the decision inbox"]')!;
  expect(button.tagName).toBe('BUTTON');
  expect(button.textContent).toBe('Review decisions');
  act(() => button.click());
  expect(host.querySelectorAll('[role="dialog"]')).toHaveLength(1);
});
it('keeps the review action outside the scrollable details', () => {
  render();
  const review = host.querySelector<HTMLElement>('[data-testid="decisions-card-review"]')!;
  expect(review.style.flexShrink).toBe('0');
  expect(review.parentElement?.getAttribute('data-testid')).toBe('decisions-card');
  expect(host.querySelector('[data-testid="decisions-card-body"]')!.contains(review)).toBe(false);
  expect(host.querySelector<HTMLElement>('[data-testid="decisions-card-top-item"]')!.style.overflowWrap).toBe('anywhere');
});
it('distinguishes loading from zero and unavailable', () => {
  fixture.data = null; render();
  expect(host.textContent).toContain('Checking decisions');
  expect(host.textContent).not.toContain('0 waiting');
  expect(host.querySelector('button')).toBeNull();
  fixture.error = true; render();
  expect(host.textContent).toContain("Couldn't load decisions");
  expect(host.textContent).not.toContain('No decisions waiting');
  act(() => host.querySelector('button')!.click());
  expect(fixture.refresh).toHaveBeenCalledOnce();
});
it('keeps blocked goals visible when the decision queue is empty', () => {
  fixture.data!.total_pending = 0; render();
  expect(host.textContent).toContain('No decisions waiting');
  expect(host.textContent).toContain('2 goals need your attention');
  expect(host.textContent).not.toContain('All clear');
  expect(host.querySelector('button')!.textContent).toBe('Open inbox');
});
it('offers a quiet empty inbox without claiming activity or inventing work', () => {
  fixture.data!.total_pending = 0; fixture.data!.goals_needing_attention = 0; render();
  expect(host.textContent).toContain('No decisions waiting');
  expect(host.textContent).not.toContain('in flight');
  expect(host.textContent).not.toContain('Review the launch plan');
});
it('honors an inbox request from another surface exactly once', () => {
  fixture.pending = true; render();
  expect(host.querySelector('[role="dialog"]')).toBeTruthy();
  expect(fixture.clear).toHaveBeenCalledOnce();
});
