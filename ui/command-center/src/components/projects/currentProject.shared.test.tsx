/**
 * @vitest-environment jsdom
 *
 * One current project, shared (J7).
 *
 * Projects, Grow and Build each tracked "which project am I looking at"
 * separately, so opening a project on one surface told the other two nothing
 * and Grow forgot its project on every mount. These pin the shared selection:
 * what Projects opens is what Grow shows, and the choice survives a remount.
 *
 * The escape hatch is pinned in `TerminalManager`'s own tests and stated in
 * the store: a terminal tab's `rootPath` is fixed at tab creation and is never
 * re-pointed by a selection change.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: { readConfig: vi.fn(() => new Promise(() => {})) },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));
vi.mock('../../lib/useGoalEvents', () => ({ useGoalEvents: () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve()) }));

import { ProjectsView } from './ProjectsView';
import { GrowView } from '../grow/GrowView';
import { useCommandCenter, CURRENT_PROJECT_KEY } from '../../lib/store';
import { apiFetch } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const apiFetchMock = vi.mocked(apiFetch);

let storage: Record<string, string> = {};
vi.stubGlobal('localStorage', {
  getItem: (k: string) => (k in storage ? storage[k] : null),
  setItem: (k: string, v: string) => { storage[k] = v; },
  removeItem: (k: string) => { delete storage[k]; },
  clear: () => { storage = {}; },
});

const project = (id: string, name: string) => ({
  id, slug: id, name, description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [],
  metadataJson: {}, createdAt: '', updatedAt: '', lastOpenedAt: '',
});

const calledUrls = () => apiFetchMock.mock.calls.map((c) => String(c[0]));
const opened = (id: string) => calledUrls().some((u) => u.startsWith(`/api/projects/${id}/`));

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  storage = {};
  apiFetchMock.mockReset();
  apiFetchMock.mockImplementation(((url: string) => {
    if (url === '/api/projects') {
      return Promise.resolve([project('p1', 'First'), project('p42', 'Target')]);
    }
    return Promise.resolve([]);
  }) as typeof apiFetch);
  useCommandCenter.setState({
    pendingProjectNavigation: null, openGrowForProject: null,
    workspaces: [], currentProjectId: null,
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(node: React.ReactElement) {
  await act(async () => { root.render(node); });
  await act(async () => { await Promise.resolve(); });
  // A project change is faded, so let the swap elapse before asserting.
  await act(async () => { await new Promise((r) => setTimeout(r, 250)); });
}

describe('one current project across the tabs', () => {
  it('what Projects opens is what Grow shows', async () => {
    useCommandCenter.setState({ pendingProjectNavigation: 'p42' });
    await mount(<ProjectsView />);
    expect(useCommandCenter.getState().currentProjectId).toBe('p42');

    act(() => root.unmount());
    apiFetchMock.mockClear();
    root = createRoot(container);
    await mount(<GrowView />);

    // Grow followed the selection instead of defaulting to the first project.
    expect(opened('p42')).toBe(true);
    expect(opened('p1')).toBe(false);
  });

  it('Grow adopts the first project only when nothing is selected anywhere', async () => {
    await mount(<GrowView />);
    expect(useCommandCenter.getState().currentProjectId).toBe('p1');
    expect(opened('p1')).toBe(true);
  });

  it('the selection is remembered across launches', async () => {
    useCommandCenter.getState().setCurrentProject('p42');
    expect(storage[CURRENT_PROJECT_KEY]).toBe('p42');
    useCommandCenter.getState().setCurrentProject(null);
    expect(CURRENT_PROJECT_KEY in storage).toBe(false);
  });

  it('a selection this list has lost is dropped, not left pointing at nothing', async () => {
    useCommandCenter.setState({ currentProjectId: 'deleted-999' });
    await mount(<ProjectsView />);
    expect(useCommandCenter.getState().currentProjectId).toBeNull();
  });
});
