/**
 * @vitest-environment jsdom
 *
 * G6 (dagK QA, 2026-09-25): "Measure now" wires `/api/growth-actions/measure`
 * (the on-demand run of the same nightly sweep) into Results, with a status
 * line and disabled-while-running — before this it did not exist at all, so
 * the only way to see whether a just-shipped fix moved anything was to wait
 * for the cron window.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));

vi.mock('../../lib/api', () => ({
  apiFetch,
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { GrowResults } from './GrowResults';
import { getThemedColors } from '../../styles/tokens';
import type { Project } from '../projects/types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const colors = getThemedColors();

const project: Project = {
  id: 'p1', slug: 'p1', name: 'GrocerySaver', description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [], metadataJson: {},
  createdAt: '2026-08-01T00:00:00Z', updatedAt: '2026-08-01T00:00:00Z',
  lastOpenedAt: '2026-08-01T00:00:00Z',
};

function resultsPayload() {
  return {
    project: null,
    fleet: {
      projects: 0, helped: 0, hindered: 0, noEffect: 0, inconclusive: 0,
      categories: [], recent: [], trend: [], byProject: [],
    },
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => { root.render(<GrowResults project={project} colors={colors} />); });
}

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

function button(): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll('[data-testid="measure-now-button"]'))[0];
  expect(found, 'no Measure now button').toBeTruthy();
  return found as HTMLButtonElement;
}

describe('Measure now', () => {
  it('is disabled while the run is in flight and re-enabled after', async () => {
    let resolveMeasure: (v: unknown) => void = () => {};
    apiFetch.mockImplementation((url: string) => {
      if (url.includes('/growth-actions/measure')) {
        return new Promise((resolve) => { resolveMeasure = resolve; });
      }
      return Promise.resolve(resultsPayload());
    });
    await render();
    await flush();

    expect(button().disabled).toBe(false);

    await act(async () => { button().click(); });
    expect(button().disabled).toBe(true);
    expect(container.querySelector('[data-testid="measure-now-status"]')?.textContent)
      .toMatch(/Measuring/);

    // Button.tsx holds its pending state for a minimum floor (MIN_PENDING_MS)
    // so a fast round trip still reads as an action — real timers, since
    // fake ones would also have to account for the polling effects this
    // component starts (usePollWhenVisible et al).
    await act(async () => {
      resolveMeasure({ actionsConsidered: 4, windowsJudged: 2, actionsCompleted: 1, errors: [] });
      await new Promise((r) => setTimeout(r, 750));
    });

    expect(button().disabled).toBe(false);
  }, 10_000);

  it('shows the sweep tally on success and refreshes the results underneath', async () => {
    apiFetch.mockImplementation((url: string) => {
      if (url.includes('/growth-actions/measure')) {
        return Promise.resolve({ actionsConsidered: 4, windowsJudged: 2, actionsCompleted: 1, errors: [] });
      }
      return Promise.resolve(resultsPayload());
    });
    await render();
    await flush();

    const before = apiFetch.mock.calls.filter((c) => String(c[0]).includes('/api/growth-results')).length;
    await act(async () => { button().click(); });
    await flush();

    expect(container.querySelector('[data-testid="measure-now-status"]')?.textContent).toContain('Checked 4 actions');
    expect(container.querySelector('[data-testid="measure-now-status"]')?.textContent).toContain('judged 2 windows');
    expect(container.querySelector('[data-testid="measure-now-status"]')?.textContent).toContain('completed 1');

    // The sweep can flip a verdict, so the tallies above it need a fresh read.
    const after = apiFetch.mock.calls.filter((c) => String(c[0]).includes('/api/growth-results')).length;
    expect(after).toBeGreaterThan(before);
  });

  it('reports a refused run rather than silently doing nothing', async () => {
    apiFetch.mockImplementation((url: string) => {
      if (url.includes('/growth-actions/measure')) {
        return Promise.reject(new Error('the daemon is busy'));
      }
      return Promise.resolve(resultsPayload());
    });
    await render();
    await flush();

    await act(async () => {
      button().click();
      await new Promise((r) => setTimeout(r, 750));
    });

    expect(container.querySelector('[data-testid="measure-now-status"]')?.textContent)
      .toContain('the daemon is busy');
    expect(button().disabled).toBe(false);
  }, 10_000);
});
