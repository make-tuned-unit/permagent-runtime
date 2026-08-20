/**
 * @vitest-environment jsdom
 *
 * Grow deep-link consume-ONCE semantics.
 *
 * `growProject` (agent open_item "grow" / the Projects "Grow this project"
 * button) stashes `openGrowForProject`; GrowView consumes it to preselect the
 * project. Before the consume-then-clear fix, the pending value stuck in the
 * store forever: every later manual Grow visit re-selected that project on
 * remount, and a repeat agent open for the SAME project was a silent no-op
 * (same store value → no re-render → effect never re-fired). These tests pin
 * the pendingProjectNavigation-style contract: apply once, then clear.
 *
 * `../../lib/api` is mocked (the SessionsList.error.test pattern) so mounting
 * GrowView touches no network; which project GrowView selected is observed
 * through the per-project sub-resource fetches it drives (posts/people/cards).
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {},
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { GrowView } from './GrowView';
import { useCommandCenter } from '../../lib/store';
import { apiFetch } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const apiFetchMock = vi.mocked(apiFetch);

const project = (id: string, name: string) => ({
  id,
  slug: id,
  name,
  description: '',
  status: 'active',
  rootPath: null,
  siteUrl: null,
  repoUrl: null,
  tags: [],
  metadataJson: {},
  createdAt: '',
  updatedAt: '',
  lastOpenedAt: '',
});

/** URLs apiFetch was called with since the last mockClear. */
const calledUrls = () => apiFetchMock.mock.calls.map((c) => String(c[0]));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetchMock.mockReset();
  apiFetchMock.mockImplementation(((url: string) => {
    if (url === '/api/projects') {
      return Promise.resolve([project('p1', 'First'), project('p42', 'Target')]);
    }
    return Promise.resolve([]); // posts / people / cards sub-resources
  }) as typeof apiFetch);
  useCommandCenter.setState({ openGrowForProject: null, workspaces: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** A project change fades the panel out BEFORE it applies, so the swap and its
 *  loading states happen while nothing is visible. Anything asserting on the
 *  newly selected project has to let that fade elapse first. */
async function settleSwap() {
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 250)); });
}

/** Mount GrowView and flush the load → select → sub-resource fetch cascade. */
async function renderGrow() {
  await act(async () => {
    root.render(<GrowView />);
  });
  // Second pass: the projects promise resolved inside the first act; flush the
  // effects that re-ran off the resulting activeId change (posts/people/cards).
  await act(async () => {
    await Promise.resolve();
  });
  // A deep link arrives as a project switch, which is faded like any other.
  await settleSwap();
}

describe('Grow deep-link is consumed exactly once', () => {
  it('applies the pending project, then clears the store seam', async () => {
    useCommandCenter.setState({ openGrowForProject: 'p42' });
    await renderGrow();

    // Applied: GrowView selected p42 (its sub-resources were fetched)…
    expect(calledUrls().some((u) => u.startsWith('/api/projects/p42/'))).toBe(true);
    // …and consumed: the seam is cleared, not stuck for the next visit.
    expect(useCommandCenter.getState().openGrowForProject).toBeNull();
  });

  it('a later manual Grow visit does NOT re-select the consumed project', async () => {
    useCommandCenter.setState({ openGrowForProject: 'p42' });
    await renderGrow();
    act(() => root.unmount());

    // Manual revisit: fresh mount, no pending deep link left behind.
    apiFetchMock.mockClear();
    root = createRoot(container);
    await renderGrow();

    // Default selection (first project), not the stale deep-link target.
    expect(calledUrls().some((u) => u.startsWith('/api/projects/p1/'))).toBe(true);
    expect(calledUrls().some((u) => u.startsWith('/api/projects/p42/'))).toBe(false);
  });

  it('growProject sets the seam and setOpenGrowForProject(null) clears it (store contract)', () => {
    // workspaces: [] makes growProject's navigateToTool a no-op (the
    // dispatchOpenItem test precedent) — this isolates the pending-state pair.
    useCommandCenter.getState().growProject('p9');
    expect(useCommandCenter.getState().openGrowForProject).toBe('p9');
    useCommandCenter.getState().setOpenGrowForProject(null);
    expect(useCommandCenter.getState().openGrowForProject).toBeNull();
  });
});

describe('Grow project request ordering', () => {
  it('does not replace the selected project posts with a stale response', async () => {
    const firstPosts = deferred<Array<{ id: string; title: string; description: string }>>();
    const secondPosts = deferred<Array<{ id: string; title: string; description: string }>>();
    apiFetchMock.mockImplementation(((url: string) => {
      if (url === '/api/projects') return Promise.resolve([project('p1', 'First'), project('p42', 'Target')]);
      if (url.includes('/p1/cards?card_type=social_post')) return firstPosts.promise;
      if (url.includes('/p42/cards?card_type=social_post')) return secondPosts.promise;
      return Promise.resolve([]);
    }) as typeof apiFetch);

    await renderGrow();
    const select = container.querySelector<HTMLSelectElement>('select[aria-label="Select project"]')!;
    await act(async () => {
      select.value = 'p42';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await settleSwap();
    await act(async () => { secondPosts.resolve([{ id: 'b', title: 'Target post', description: '' }]); });

    const calendar = Array.from(container.querySelectorAll('[role="tab"]')).find((button) => button.textContent === 'Calendar')!;
    await act(async () => { calendar.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(container.textContent).toContain('Target post');

    await act(async () => { firstPosts.resolve([{ id: 'a', title: 'Stale first post', description: '' }]); });
    expect(container.textContent).toContain('Target post');
    expect(container.textContent).not.toContain('Stale first post');
  });

  /// Switching used to be a hard cut: every panel dropped to its loading state
  /// in one frame and sprang back when the slowest request landed. The panel
  /// now fades out first, so the swap happens unseen.
  it('fades the panel out before the project changes, and back in after', async () => {
    await renderGrow();
    const panel = () => container.querySelector<HTMLDivElement>('[role="tabpanel"]')!;
    expect(panel().getAttribute('aria-busy')).toBe('false');

    const select = container.querySelector<HTMLSelectElement>('select[aria-label="Select project"]')!;
    await act(async () => {
      select.value = 'p42';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });

    // Mid-swap: invisible and announced as busy, so the loading states the new
    // project is about to show are never seen.
    expect(panel().getAttribute('aria-busy')).toBe('true');
    expect(panel().style.opacity).toBe('0');
    expect(panel().style.transition).toContain('opacity');
    // …but the control the user just used holds THEIR choice throughout. A
    // select that springs back for the length of the fade reads as the app
    // arguing with the click.
    expect(select.value).toBe('p42');

    await settleSwap();
    expect(panel().getAttribute('aria-busy')).toBe('false');
    expect(panel().style.opacity).toBe('1');
  });
});
