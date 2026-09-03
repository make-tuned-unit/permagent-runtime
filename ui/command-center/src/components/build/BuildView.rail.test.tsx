/**
 * @vitest-environment jsdom
 *
 * Build's header carried five flat grey bars, permanently, with no label, no
 * `title`, no `aria-label` and no adjacent text. On an idle tab — which is most
 * of the time — they were a shape that never changed and never said anything:
 * decoration wearing an instrument's clothes. Worse for anyone who eventually
 * caught them moving, since nothing had ever said what they measured.
 *
 * Now they appear when there is progress to show and name what they are
 * counting while they do; and the idle header says what the tab is for instead
 * of only that nothing is happening.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

type Flight = { id: string; title: string; progress: number };
const dashboard = vi.hoisted(() => ({
  data: { agent: { name: 'Aria' }, in_flight: [] } as {
    agent: { name: string };
    in_flight: Array<{ id: string; title: string; progress: number }>;
  },
}));

function withFlight(...in_flight: Flight[]) {
  dashboard.data = { agent: { name: 'Aria' }, in_flight };
}

vi.mock('../dashboard/useDashboard', () => ({
  useDashboard: () => ({ data: dashboard.data, loading: false, error: false }),
}));
vi.mock('../browser', () => ({ Browser: () => null }));
vi.mock('./ProjectChip', () => ({ ProjectChip: () => null }));
vi.mock('./CostStatusline', () => ({ CostStatusline: () => null }));
vi.mock('../terminal/TerminalManager', () => {
  const React = require('react');
  const TerminalManager = React.forwardRef((_props: unknown, ref: unknown) => {
    React.useImperativeHandle(ref, () => ({
      createProjectTab: () => {},
      getActiveTab: () => ({ id: 'tab-1', label: 'Terminal', sessionId: null }),
      getAllTabs: () => [],
      killTab: async () => {},
    }));
    return null;
  });
  return { TerminalManager };
});

import { BuildView } from './BuildView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// The split panes measure themselves on mount; jsdom has no ResizeObserver.
class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = FakeResizeObserver;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  withFlight();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => { root.render(<BuildView />); });
}

describe('Build progress rail', () => {
  it('is not on screen when there is no progress to report', async () => {
    await render();
    expect(container.querySelector('[data-testid="build-progress-rail"]')).toBeNull();
  });

  it('says what it is counting once it appears', async () => {
    withFlight({ id: 'g1', title: 'Refactor the parser', progress: 0.5 });
    await render();
    const rail = container.querySelector('[data-testid="build-progress-rail"]') as HTMLElement;
    expect(rail).not.toBeNull();
    expect(rail.textContent).toContain('Step 3 of 5');
    expect(rail.getAttribute('aria-label')).toContain('Refactor the parser');
    expect(rail.getAttribute('aria-valuenow')).toBe('3');
    // A shape with no words is not an instrument — the visible "Step N of 5"
    // label (above) and the glass tip on focus both name what is being counted.
    act(() => { (rail.parentElement as HTMLElement).focus(); });
    expect(document.querySelector('[role="tooltip"]')?.textContent).toMatch(/Step 3 of 5/);
  });

  it('says what an idle Build tab is for', async () => {
    await render();
    expect(container.querySelector('[data-testid="view-header"]')!.textContent)
      .toContain('the terminal below runs your coding agent');
  });
});
