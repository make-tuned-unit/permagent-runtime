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

const harness = vi.hoisted(() => ({ runs: [] as Array<Record<string, unknown>> }));
vi.mock('../../lib/api', async importOriginal => ({
  ...(await importOriginal<typeof import('../../lib/api')>()),
  api: {
    getActiveHarnessRuns: () => Promise.resolve(harness.runs),
    getHarnessRunHistory: () => Promise.resolve([]),
  },
}));

/** The dashboard lists every recently-touched session; only a harness run with
 *  a turn in flight is a Build session, so `withFlight` registers both. */
function withFlight(...in_flight: Flight[]) {
  dashboard.data = { agent: { name: 'Aria' }, in_flight };
  harness.runs = in_flight.map(f => ({ runId: `run-${f.id}`, sessionId: f.id, promptTitle: f.title, status: 'running', elapsedMs: 0 }));
}
/** A plain chat session: on the dashboard, but no harness run. */
function withChatSessionOnly(...in_flight: Flight[]) {
  dashboard.data = { agent: { name: 'Aria' }, in_flight };
  harness.runs = [];
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

  it('a newly created chat session does not change the Build header (G4)', async () => {
    withChatSessionOnly({ id: '20261013_1', title: 'Session 20261013_1', progress: 0 });
    await render();
    const header = container.querySelector('[data-testid="view-header"]')!.textContent!;
    expect(header).toContain('Aria · idle');
    expect(header).toContain('the terminal below is a plain shell');
    expect(header).not.toContain('thinking');
    expect(header).not.toContain('Session 20261013_1');
    expect(header).not.toContain('Take over');
    expect(container.querySelector('[data-testid="build-progress-rail"]')).toBeNull();
  });

  it('a queued or finished harness run is not a turn in flight', async () => {
    withChatSessionOnly();
    harness.runs = [{ runId: 'r', sessionId: 's', promptTitle: 'Queued job', status: 'queued', elapsedMs: 0 },
      { runId: 'r2', sessionId: 's2', promptTitle: 'Done job', status: 'succeeded', elapsedMs: 9 }];
    await render();
    const header = container.querySelector('[data-testid="view-header"]')!.textContent!;
    expect(header).not.toContain('thinking');
    expect(header).not.toContain('Take over');
  });

  it('says what an idle Build tab is for, truthfully — the terminal is a plain shell until a project is launched (B1)', async () => {
    await render();
    const text = container.querySelector('[data-testid="view-header"]')!.textContent;
    expect(text).toContain('the terminal below is a plain shell');
    expect(text).not.toContain('the terminal below runs your coding agent');
  });
});
