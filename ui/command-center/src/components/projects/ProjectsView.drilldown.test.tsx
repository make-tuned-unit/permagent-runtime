/**
 * @vitest-environment jsdom
 *
 * Voice/agent drill-into-a-specific-project navigation (#266, follow-up to the
 * #264 wiring that was merged "NOT yet voice-verified").
 *
 * The agent resolves a project id daemon-side (project_resolve → the LIVE DB)
 * and hands it to the frontend via the `pendingProjectNavigation` store seam;
 * ProjectsView consumes it to open that project's board. In the voice flow the
 * drill-in fires SECONDS after the resolve (held behind the narration audio),
 * against this view's ≤5s-old polled snapshot — so the resolved id can be ahead
 * of the cache. The original consumer cleared the pending id whenever the target
 * wasn't in its current snapshot, silently dropping the navigation: the classic
 * #266 "reports done but the view doesn't change", flaky on cache timing.
 *
 * These pin the resilient contract:
 *   - a present target opens immediately and the seam is consumed once;
 *   - a target missing from the stale snapshot self-heals via a forced reload
 *     rather than being dropped;
 *   - a genuinely-absent id is dropped after ONE re-check (no infinite reload).
 *
 * `../../lib/api` is mocked (the GrowView.consume pattern) so mounting touches
 * no network; which project is open is observed through the per-project board
 * fetches ProjectOverview drives. Tauri IPC + the goal-event socket are stubbed
 * so the DOM-only / native-only side effects never run under the node test env.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  // StrixFindingsPanel reads `strix_enabled` on mount (honest empty state);
  // an unresolved read keeps the panel in its "unknown" state, which is fine.
  api: { readConfig: vi.fn(() => new Promise(() => {})) },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractSystemNotice: vi.fn(() => null),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));
// The board views subscribe to a goal-event WebSocket on mount; stub it so the
// test never opens a socket. Tauri core is absent under node — stub invoke so
// the fire-and-forget ProjectSelected activity emit is a no-op, not a throw.
vi.mock('../../lib/useGoalEvents', () => ({ useGoalEvents: () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve()) }));

import { ProjectsView } from './ProjectsView';
import { PERSONAL_ID, type Project } from './types';
import { useCommandCenter } from '../../lib/store';
import { apiFetch } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const apiFetchMock = vi.mocked(apiFetch);

// jsdom here doesn't provide localStorage; stub a minimal in-memory one (the
// deleteSession.store.test pattern) so ProjectsView's last-opened persistence
// runs instead of throwing.
let storage: Record<string, string> = {};
vi.stubGlobal('localStorage', {
  getItem: (k: string) => (k in storage ? storage[k] : null),
  setItem: (k: string, v: string) => { storage[k] = v; },
  removeItem: (k: string) => { delete storage[k]; },
  clear: () => { storage = {}; },
});

const project = (id: string, name: string): Project => ({
  id, slug: id, name, description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [],
  metadataJson: {}, createdAt: '', updatedAt: '', lastOpenedAt: '',
});

const calledUrls = () => apiFetchMock.mock.calls.map((c) => String(c[0]));
const openedProject = (id: string) => calledUrls().some((u) => u.startsWith(`/api/projects/${id}/`));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

/** Serve `/api/projects` from a mutable list; sub-resources return []. */
function serveProjects(getList: () => ReturnType<typeof project>[]) {
  apiFetchMock.mockImplementation(((url: string) => {
    if (url === '/api/projects') return Promise.resolve(getList());
    return Promise.resolve([]);
  }) as typeof apiFetch);
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  storage = {};
  apiFetchMock.mockReset();
  serveProjects(() => [project('p1', 'First'), project('p42', 'Target')]);
  // The last-opened project now lives in the store (one selection shared with
  // Grow and Build, J7), which reads localStorage once at creation — so a test
  // seeds the selection through the store rather than through storage.
  useCommandCenter.setState({ pendingProjectNavigation: null, workspaces: [], currentProjectId: null });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** Mount + flush the load → consume → board-fetch cascade (a few microtask passes). */
async function renderView() {
  await act(async () => { root.render(<ProjectsView />); });
  for (let i = 0; i < 4; i++) await act(async () => { await Promise.resolve(); });
}

async function flush() {
  for (let i = 0; i < 4; i++) await act(async () => { await Promise.resolve(); });
}

describe('#266 voice drill-into-specific-project', () => {
  it('opens the target project and consumes the seam exactly once', async () => {
    useCommandCenter.setState({ pendingProjectNavigation: 'p42' });
    await renderView();

    expect(openedProject('p42')).toBe(true);
    expect(useCommandCenter.getState().pendingProjectNavigation).toBeNull();
  });

  it('switches boards when a drill-in arrives while another project is open', async () => {
    useCommandCenter.setState({ currentProjectId: 'p1' }); // the app's current project
    await renderView();
    expect(openedProject('p1')).toBe(true);

    apiFetchMock.mockClear();
    await act(async () => { useCommandCenter.setState({ pendingProjectNavigation: 'p42' }); });
    await flush();

    expect(openedProject('p42')).toBe(true);
    expect(useCommandCenter.getState().pendingProjectNavigation).toBeNull();
  });

  it('self-heals a target missing from the stale snapshot via a forced reload (the #266 fix)', async () => {
    // The first snapshot lags the daemon (p77 resolvable server-side but not yet
    // in this view's polled list); the forced reload returns the up-to-date list
    // the daemon actually has. The drill-in must ride that reload through to the
    // board, not be silently dropped on the stale snapshot.
    let projectsFetches = 0;
    apiFetchMock.mockImplementation(((url: string) => {
      if (url === '/api/projects') {
        projectsFetches += 1;
        return Promise.resolve(
          projectsFetches === 1
            ? [project('p1', 'First')] // stale snapshot: no p77 yet
            : [project('p1', 'First'), project('p77', 'Fresh')], // reload has it
        );
      }
      return Promise.resolve([]);
    }) as typeof apiFetch);

    useCommandCenter.setState({ pendingProjectNavigation: 'p77' });
    await renderView();

    // Resolved via the forced reload rather than dropped on the stale snapshot.
    expect(openedProject('p77')).toBe(true);
    expect(useCommandCenter.getState().pendingProjectNavigation).toBeNull();
    expect(projectsFetches).toBeGreaterThanOrEqual(2); // initial + forced re-check
  });

  it('drops a genuinely stale/deleted id after one re-check, without looping', async () => {
    serveProjects(() => [project('p1', 'First'), project('p42', 'Target')]);
    useCommandCenter.setState({ pendingProjectNavigation: 'gone-999' });
    await renderView();

    // One forced reload confirmed absence → the unresolvable id is cleared, and
    // no project board was opened.
    expect(useCommandCenter.getState().pendingProjectNavigation).toBeNull();
    expect(openedProject('gone-999')).toBe(false);

    // The reload settled: /api/projects was fetched a bounded number of times
    // (initial load + the single forced re-check), never in a runaway loop.
    const projectsListFetches = calledUrls().filter((u) => u === '/api/projects').length;
    expect(projectsListFetches).toBeLessThanOrEqual(3);
  });
});

describe('Projects request validation and ordering', () => {
  it('discards an older project list that resolves after a newer refresh', async () => {
    const oldRequest = deferred<ReturnType<typeof project>[]>();
    const newRequest = deferred<ReturnType<typeof project>[]>();
    apiFetchMock
      .mockImplementationOnce(() => oldRequest.promise)
      .mockImplementationOnce(() => newRequest.promise);

    await act(async () => { root.render(<ProjectsView />); });
    await act(async () => { window.dispatchEvent(new Event('focus')); });
    await act(async () => { newRequest.resolve([project('new', 'Current project')]); });
    expect(container.textContent).toContain('Current project');

    await act(async () => { oldRequest.resolve([project('old', 'Deleted project')]); });
    expect(container.textContent).toContain('Current project');
    expect(container.textContent).not.toContain('Deleted project');
  });

  it('routes a non-array project response to the existing error state', async () => {
    apiFetchMock.mockResolvedValueOnce({ projects: [] } as never);
    await renderView();

    expect(container.textContent).toContain("Couldn't load projects");
  });
});


it('searches shelved projects and changes status through the existing route without opening the project', async () => {
  serveProjects(() => [project('p1', 'First'), { ...project('p42', 'Garden journal'), status: 'paused' }]);
  await renderView();
  const search = container.querySelector<HTMLInputElement>('input[aria-label="Search projects"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(search, 'garden');
    search.dispatchEvent(new Event('input', { bubbles: true }));
  });
  expect(container.querySelector('[aria-label="Open project First"]')).toBeNull();
  expect(container.querySelector('[aria-label="Open project Garden journal"]')).toBeTruthy();
  const status = container.querySelector<HTMLSelectElement>('[aria-label="Status for Garden journal"]')!;
  await act(async () => { status.value = 'active'; status.dispatchEvent(new Event('change', { bubbles: true })); });
  expect(apiFetchMock.mock.calls.some(([url, opts]) => url === '/api/projects/p42' && opts?.method === 'PATCH' && JSON.parse(opts.body as string).status === 'active')).toBe(true);
  expect(useCommandCenter.getState().currentProjectId).toBeNull();
});

it('disables a status selector until its PATCH settles, preventing duplicate submissions', async () => {
  const patchResponse = deferred<unknown>();
  let patchCalls = 0;
  apiFetchMock.mockImplementation(((url: string, options?: RequestInit) => {
    if (url === '/api/projects') return Promise.resolve([project('p42', 'Garden journal')]);
    if (url === '/api/projects/p42' && options?.method === 'PATCH') {
      patchCalls += 1;
      return patchResponse.promise;
    }
    return Promise.resolve([]);
  }) as typeof apiFetch);
  await renderView();

  const status = container.querySelector<HTMLSelectElement>('[aria-label="Status for Garden journal"]')!;
  await act(async () => {
    status.value = 'paused';
    status.dispatchEvent(new Event('change', { bubbles: true }));
  });
  expect(status.disabled).toBe(true);
  expect(status.getAttribute('aria-busy')).toBe('true');

  await act(async () => {
    status.value = 'archived';
    status.dispatchEvent(new Event('change', { bubbles: true }));
  });
  expect(patchCalls).toBe(1);

  await act(async () => { patchResponse.resolve({}); });
  expect(status.disabled).toBe(false);
  expect(status.hasAttribute('aria-busy')).toBe(false);
});

it('groups projects by status and keeps each bounded card control inside its own card', async () => {
  const personal = { ...project(PERSONAL_ID, 'Personal'), tags: ['default'] };
  const current = { ...project('active-id', 'Current work'), description: 'A focused project summary',
    tags: ['one', 'two', 'three', 'four', 'five'] };
  const paused = { ...project('paused-id', 'Paused work'), status: 'paused' };
  const archived = { ...project('archived-id', 'Archived work'), status: 'archived' };
  serveProjects(() => [personal, current, paused, archived]);
  await renderView();

  expect(container.querySelector('#projects-active-heading')?.textContent).toBe('Active');
  const activeCard = container.querySelector<HTMLElement>('[aria-label="Open project Current work"]')!
    .closest<HTMLElement>('.project-collection-card')!;
  const openControl = activeCard.querySelector('[role="button"]')!;
  const statusControl = activeCard.querySelector<HTMLSelectElement>('[aria-label="Status for Current work"]')!;
  expect(statusControl.closest('.project-collection-card')).toBe(activeCard);
  expect(openControl.contains(statusControl)).toBe(false);
  expect(activeCard.querySelectorAll('.project-collection-tag')).toHaveLength(4);
  expect(activeCard.querySelector('.project-collection-tag-more')?.textContent).toBe('+2');
  expect(container.querySelector('[aria-label="Status for Personal"]')).toBeNull();
  expect(container.querySelector('[aria-label="Open project Personal"]')?.textContent).toContain('DEFAULT');

  const filter = container.querySelector<HTMLSelectElement>('[aria-label="Filter projects by status"]')!;
  await act(async () => {
    filter.value = 'paused';
    filter.dispatchEvent(new Event('change', { bubbles: true }));
  });
  expect(container.querySelector('[aria-label="Open project Current work"]')).toBeNull();
  expect(container.querySelector('[aria-label="Open project Paused work"]')).toBeTruthy();
  expect(container.querySelector('[aria-label="Open project Archived work"]')).toBeNull();
  expect(container.textContent).toContain('1 of 4 projects');
});
