/**
 * @vitest-environment jsdom
 *
 * `project_selected` activity events must carry the id of the chat session
 * the selection led to, when there is one. Opening a project's board from
 * this surface starts no chat — no session exists yet — so `emitProjectSelected`
 * must still send `session_id: null` EXPLICITLY (never omit the key, and never
 * fabricate an id) alongside the canonical `project:<slug>` project id.
 *
 * Mirrors the mocking pattern in `ProjectsView.drilldown.test.tsx`: `../../lib/api`
 * is mocked so mounting touches no network, and `@tauri-apps/api/core` is
 * stubbed so the fire-and-forget emit is observable instead of throwing under
 * node (no real Tauri context).
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
const invoke = vi.fn((_cmd: string, _payload?: Record<string, unknown>) => Promise.resolve());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { ProjectsView } from './ProjectsView';
import { useCommandCenter } from '../../lib/store';
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

const project = (id: string, slug: string, name: string) => ({
  id, slug, name, description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [],
  metadataJson: {}, createdAt: '', updatedAt: '', lastOpenedAt: '',
});

function serveProjects(list: ReturnType<typeof project>[]) {
  apiFetchMock.mockImplementation(((url: string) => {
    if (url === '/api/projects') return Promise.resolve(list);
    return Promise.resolve([]);
  }) as typeof apiFetch);
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  storage = {};
  invoke.mockClear();
  apiFetchMock.mockReset();
  serveProjects([project('p1', 'permagent', 'Permagent')]);
  useCommandCenter.setState({ pendingProjectNavigation: null, workspaces: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function renderView() {
  await act(async () => { root.render(<ProjectsView />); });
  for (let i = 0; i < 4; i++) await act(async () => { await Promise.resolve(); });
}

describe('project_selected — session id (or its explicit absence)', () => {
  it('opening a project board with no chat session sends session_id: null explicitly', async () => {
    await renderView();

    const card = container.querySelector('[aria-label="Open project Permagent"]') as HTMLElement;
    expect(card).toBeTruthy();
    await act(async () => { card.click(); });
    await act(async () => { await Promise.resolve(); });

    const call = invoke.mock.calls.find(([cmd]) => cmd === 'emit_activity');
    expect(call).toBeTruthy();
    const payload = call![1] as Record<string, unknown>;
    expect(payload.event_type).toBe('project_selected');
    // Explicit null — the key must be present, never omitted, and never a
    // fabricated id for a selection that started no chat.
    expect(payload).toHaveProperty('session_id', null);
    expect(payload.project_id).toBe('project:permagent');
  });

  it('a drill-in navigation (agent/voice) also sends session_id: null explicitly', async () => {
    useCommandCenter.setState({ pendingProjectNavigation: 'p1' });
    await renderView();

    const call = invoke.mock.calls.find(([cmd]) => cmd === 'emit_activity');
    expect(call).toBeTruthy();
    const payload = call![1] as Record<string, unknown>;
    expect(payload).toHaveProperty('session_id', null);
    expect(payload.project_id).toBe('project:permagent');
  });
});
