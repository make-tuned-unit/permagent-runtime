/** @vitest-environment jsdom */
/**
 * QA walk 2026-09-24: Home's Growth card read "13 measured" beside
 * Helped 0 / Hindered 0 / No change 0 and two flat sparklines.
 *
 * Not a wiring bug: `/api/growth-results` counts every judged window, and
 * `tally_verdicts` (growth_actions.rs) files `inconclusive` and `confounded`
 * under `inconclusive` — which the card summed into "measured" but never
 * showed. All 13 were inconclusive. The card must say so, not show zeros.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const apiFetch = vi.fn();
vi.mock('../../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));
vi.mock('../../../lib/store', () => ({
  useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({ setOpenGrowLens: () => {}, growProject: () => {} }),
  navigateToTool: () => {},
}));

import { GrowthResultsCard, growthSummary } from './GrowthResultsCard';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  apiFetch.mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const zeroWeeks = Array.from({ length: 12 }, (_, i) => ({
  week: `2026-07-${String(i + 1).padStart(2, '0')}`, helped: 0, hindered: 0, noEffect: 0, net: 0, cumulativeNet: 0,
}));

function fleet(over: Record<string, unknown>) {
  return {
    project: null,
    fleet: {
      projects: 3, helped: 0, hindered: 0, noEffect: 0, inconclusive: 13,
      categories: [], recent: [], trend: zeroWeeks,
      byProject: [
        { projectId: 'p1', projectName: 'Venture site', helped: 0, hindered: 0, noEffect: 0, points: zeroWeeks },
      ],
      ...over,
    },
  };
}

async function render() {
  await act(async () => { root.render(<GrowthResultsCard />); });
  await act(async () => { await Promise.resolve(); });
}

it('all-inconclusive: shows the inconclusive count and says why, not three zeros and flat lines', async () => {
  apiFetch.mockResolvedValue(fleet({}));
  await render();
  const text = container.textContent ?? '';
  expect(text).toContain('13 measured · 3 projects');
  expect(text).toContain('Inconclusive13');
  expect(container.querySelector('[data-testid="growth-inconclusive-note"]')?.textContent)
    .toContain('all 13 measured windows were inconclusive');
  // The flat 12-week fleet line is gone; the project row says what it knows.
  expect(text).not.toContain('All projects');
  expect(text).toContain('inconclusive so far');
  expect(text).not.toContain('0 helped');
});

it('decisive verdicts keep the trend line and per-project counts', async () => {
  const weeks = zeroWeeks.map((w, i) => (i === 11 ? { ...w, helped: 2, net: 2, cumulativeNet: 2 } : w));
  apiFetch.mockResolvedValue(fleet({
    helped: 2, inconclusive: 1, trend: weeks,
    byProject: [{ projectId: 'p1', projectName: 'Venture site', helped: 2, hindered: 0, noEffect: 0, points: weeks }],
  }));
  await render();
  const text = container.textContent ?? '';
  expect(container.querySelector('[data-testid="growth-inconclusive-note"]')).toBeNull();
  expect(text).toContain('All projects');
  expect(text).toContain('2 helped');
  expect(text).toContain('3 measured');
});

it('growthSummary counts inconclusive into measured and flags the all-inconclusive case', () => {
  expect(growthSummary({ helped: 0, hindered: 0, noEffect: 0, inconclusive: 13, projects: 3 }))
    .toEqual({ measured: 13, decisive: 0, allInconclusive: true, right: '13 measured · 3 projects' });
  expect(growthSummary({ helped: 1, hindered: 0, noEffect: 0, inconclusive: 0, projects: 1 }).allInconclusive).toBe(false);
  expect(growthSummary({ helped: 0, hindered: 0, noEffect: 0, inconclusive: 0, projects: 0 }).right).toBeUndefined();
});
