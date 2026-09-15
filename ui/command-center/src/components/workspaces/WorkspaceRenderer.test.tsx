/** @vitest-environment jsdom */
// The `world` tool is the Solar Forum. This pins that choice and the one
// rollback lever back to the original WorldView, because "which World ships"
// is otherwise a single line nobody notices changing.
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import type { ReactNode } from 'react';
import { LEGACY_WORLD_KEY } from '../world/forum/worldRoute';

const store = vi.hoisted(() => ({
  state: {
    workspaces: [{ id: 'ws-1', layoutJson: { type: 'panel', tool: 'world' } }],
    updateWorkspaceLayout: vi.fn(),
  } as Record<string, unknown>,
}));

vi.mock('../../lib/store', () => ({
  useCommandCenter: (selector: (s: unknown) => unknown) => selector(store.state),
}));
vi.mock('../../styles/useTheme', () => ({ useTheme: () => ({ colors: {} }) }));
vi.mock('../common/ErrorBoundary', () => ({
  ErrorBoundary: ({ children }: { children: ReactNode }) => <>{children}</>,
}));
// Both World candidates are stubbed: this test is about which one the route
// picks, not about Three.js.
vi.mock('../world/WorldView', () => ({ WorldView: () => <div data-testid="legacy-world" /> }));
vi.mock('../world/forum/ForumAppView', () => ({ ForumAppView: () => <div data-testid="forum-world" /> }));
vi.mock('../chat/ChatView', () => ({ ChatView: () => null }));
vi.mock('../skills/SkillsPanel', () => ({ SkillsPanel: () => null }));
vi.mock('../terminal/TerminalManager', () => ({ TerminalManager: () => null }));
vi.mock('../browser', () => ({ Browser: () => null }));
vi.mock('../trace/ExecutionTrace', () => ({ ExecutionTrace: () => null }));
vi.mock('../brain/BrainView', () => ({ BrainView: () => null }));
vi.mock('../dashboard/Dashboard', () => ({ Dashboard: () => null }));
vi.mock('../build/BuildView', () => ({ BuildView: () => null }));
vi.mock('../grow/GrowView', () => ({ GrowView: () => null }));
vi.mock('../finance/FinanceView', () => ({ FinanceView: () => null }));
vi.mock('../automate/AutomateView', () => ({ AutomateView: () => null }));
vi.mock('../projects/ProjectsView', () => ({ ProjectsView: () => null }));
vi.mock('../people/PeopleView', () => ({ PeopleView: () => null }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let host: HTMLDivElement | null = null;

// `lazy()` is created once per module instance and caches its resolved
// component, so each case needs a fresh module graph to re-read the flag.
async function renderWorldPanel() {
  vi.resetModules();
  const { WorkspaceRenderer } = await import('./WorkspaceRenderer');
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root!.render(<WorkspaceRenderer workspaceId="ws-1" />); });
  // The route is a chain of dynamic imports (worldRoute, then the chosen
  // World), so Suspense needs more than one flush before it commits.
  for (let i = 0; i < 10 && !host.querySelector('[data-testid]'); i++) {
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
  }
  return host;
}

beforeEach(() => { localStorage.clear(); });
afterEach(() => {
  if (root) act(() => root!.unmount());
  host?.remove();
  root = null; host = null;
  localStorage.clear();
});

it('renders the Solar Forum for a world panel by default', async () => {
  const panel = await renderWorldPanel();
  expect(panel.querySelector('[data-testid="forum-world"]')).not.toBeNull();
  expect(panel.querySelector('[data-testid="legacy-world"]')).toBeNull();
});

it('falls back to the legacy WorldView when the rollback flag is set', async () => {
  localStorage.setItem(LEGACY_WORLD_KEY, '1');
  const panel = await renderWorldPanel();
  expect(panel.querySelector('[data-testid="legacy-world"]')).not.toBeNull();
  expect(panel.querySelector('[data-testid="forum-world"]')).toBeNull();
});

it('ignores a stored value that is not the rollback flag', async () => {
  localStorage.setItem(LEGACY_WORLD_KEY, 'true');
  const panel = await renderWorldPanel();
  expect(panel.querySelector('[data-testid="forum-world"]')).not.toBeNull();
});
