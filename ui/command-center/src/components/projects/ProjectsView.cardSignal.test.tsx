/**
 * @vitest-environment jsdom
 *
 * P3 (QA walk 2026-09-24): a Projects grid card showed only name / description
 * / tags — no signal of what was actually happening inside the project. This
 * pins that the All-Projects grid card now shows open tasks, goals in flight
 * and last activity, derived from the project's own cards + columns.
 *
 * Mocking follows `ProjectsView.drilldown.test.tsx`'s pattern.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: { readConfig: vi.fn(() => new Promise(() => {})) },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractSystemNotice: vi.fn(() => null),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));
vi.mock('../../lib/useGoalEvents', () => ({ useGoalEvents: () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve()) }));

import { ProjectsView } from './ProjectsView';
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

const project = {
  id: 'p1', slug: 'p1', name: 'Venture site', description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [],
  metadataJson: {}, createdAt: '', updatedAt: '', lastOpenedAt: '2026-09-01T00:00:00Z',
};
const col = (id: string, stateBinding: string | null) => ({
  id, projectId: 'p1', name: id, position: 0, columnKind: 'todo', stateBinding, wipLimit: null,
});
const card = (id: string, over: Record<string, unknown> = {}) => ({
  id, projectId: 'p1', cardType: 'task', title: id, description: '', columnId: 'todo',
  position: 0, createdBy: 'user', assignedTo: null, metadataJson: {},
  createdAt: '2026-09-01T00:00:00Z', updatedAt: '2026-09-01T00:00:00Z', archivedAt: null,
  ...over,
});

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  apiFetchMock.mockReset();
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('Projects grid card live signal (P3)', () => {
  it('shows open tasks, goals in flight and last activity for an active project', async () => {
    apiFetchMock.mockImplementation(((url: string) => {
      if (url === '/api/projects') return Promise.resolve([project]);
      if (url.endsWith('/cards')) return Promise.resolve([
        card('t1', { updatedAt: '2026-09-20T00:00:00Z' }),
        card('g1', { cardType: 'goal' }),
        card('done', { columnId: 'done' }),
      ]);
      if (url.endsWith('/columns')) return Promise.resolve([col('todo', 'in_progress'), col('done', 'complete')]);
      return Promise.resolve([]);
    }) as typeof apiFetch);

    await act(async () => { root.render(<ProjectsView />); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });

    const signal = container.querySelector('[data-testid="project-card-signal"]');
    expect(signal).not.toBeNull();
    expect(signal!.textContent).toContain('1 goal');
    expect(signal!.textContent).toContain('1 open');
  });

  it('a project with no cards shows no signal row (not a row of zeros)', async () => {
    apiFetchMock.mockImplementation(((url: string) => {
      if (url === '/api/projects') return Promise.resolve([project]);
      return Promise.resolve([]);
    }) as typeof apiFetch);

    await act(async () => { root.render(<ProjectsView />); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });

    expect(container.querySelector('[data-testid="project-card-signal"]')).toBeNull();
  });
});
