/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { Dashboard } from './Dashboard';
import { DEFAULT_LAYOUT, type DashboardLayoutData } from './useLayout';
import { isDefaultHomeLayout } from './homeFirstAction';
import { useCommandCenter } from '../../lib/store';
import { setTheme } from '../../styles/tokens';

const fixture = vi.hoisted(() => ({ sessions: 0, memories: 0, layout: null as DashboardLayoutData | null, persist: vi.fn(), loaded: true }));
vi.mock('../../lib/api', () => ({ api: { setActiveWorkspace: vi.fn(async () => {}) }, apiFetch: vi.fn(async () => ({
  learn_next: [{ id: 'brain', display_name: 'Brain', what_it_does: 'Recall your memories.', tab: 'brain' }],
  totals: { used: 0, teachable: 8 },
})) }));
vi.mock('./useDashboard', () => ({ useDashboard: () => ({ data: {
  agent: { name: 'Agent', state: 'idle', active_count: 0, summary: '' },
  stats: { sessions_total: fixture.sessions, sessions_today: 0, memory_count: fixture.memories, memory_delta_today: 0 },
  in_flight: [], recent: [],
}, loading: false, error: false, failing: false, lastOkAt: 0 }) }));
vi.mock('./useLayout', async importOriginal => ({ ...await importOriginal<typeof import('./useLayout')>(),
  useLayout: () => ({ layout: fixture.layout, layoutLoaded: fixture.loaded, persistLayout: fixture.persist }),
}));
vi.mock('../../lib/useLiveGoals', () => ({ useLiveGoals: () => ({ goals: [], activeCount: 0 }) }));
vi.mock('../../lib/useDueTodos', () => ({ useDueTodos: () => ({ todos: [] }) }));
vi.mock('../sidebar/NavAgentStatus', () => ({ HeaderAgentStatus: () => null }));
vi.mock('./Echo', () => ({ Echo: () => null }));
vi.mock('./dashboardLayoutPreviewEvent', () => ({ useDashboardLayoutPreviewEvent: () => null }));
vi.mock('./cards/useCardRegistry', () => ({ useCardRegistry: () => ({ registry: {}, status: 'ready' }) }));

let host: HTMLDivElement;
let root: Root;
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
async function render() { await act(async () => { root.render(<Dashboard />); }); }
function button(label: string) { return [...host.querySelectorAll('button')].find(b => b.textContent === label)!; }
beforeEach(() => {
  localStorage.clear(); fixture.sessions = 0; fixture.memories = 0; fixture.loaded = true;
  fixture.layout = structuredClone(DEFAULT_LAYOUT); fixture.persist.mockClear();
  useCommandCenter.setState({ currentModel: null, providersError: false, loadProviders: vi.fn(async () => {}),
    chatResolvedRoute: { provider: '', model: '', source: 'unconfigured' }, activePanel: 'chat', activeWorkspaceId: 'home', workspaces: [{ id: 'world', name: 'World', icon: '', sortOrder: 0, isDefault: true, layoutJson: { type: 'panel', tool: 'world', config: {} } }] });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); act(() => setTheme('dark')); });

describe('Home first action', () => {
  it('confirms model configuration when Home is opened before Chat', async () => {
    const loadProviders = vi.fn(async () => { useCommandCenter.setState({ chatResolvedRoute: { provider: '', model: '', source: 'unconfigured' } }); });
    useCommandCenter.setState({ chatResolvedRoute: null, loadProviders });
    await render();
    expect(loadProviders).toHaveBeenCalledOnce();
    expect(button('Choose a model')).toBeTruthy();
  });
  it('leads with model setup, keeps a useful Learn next banner and opens World without a model', async () => {
    await render();
    expect(button('Choose a model')).toBeTruthy();
    expect(host.querySelectorAll('[data-testid^="dashboard-cell-"]')).toHaveLength(0);
    expect(host.textContent).toContain('No runs or memories yet.');
    expect(host.querySelector('[data-testid="home-banner-learn-next"]')?.textContent).toContain('World is ready to explore');
    expect(host.textContent).not.toContain('tried Brain');
    act(() => button('Choose a model').click());
    expect(useCommandCenter.getState().pendingSettingsSection).toBe('models');
    expect(useCommandCenter.getState().activePanel).toBe('settings');
    act(() => button('Explore World').click());
    expect(useCommandCenter.getState().activeWorkspaceId).toBe('world');
    expect(fixture.persist).not.toHaveBeenCalled();
  });
  it.each(['model', 'runs', 'memories', 'unknown', 'loading'] as const)('does not replace a %s profile with the empty summary', async state => {
    if (state === 'model') useCommandCenter.setState({ currentModel: 'selected-model' });
    if (state === 'runs') fixture.sessions = 2;
    if (state === 'memories') fixture.memories = 2;
    if (state === 'unknown') useCommandCenter.setState({ chatResolvedRoute: null });
    if (state === 'loading') fixture.loaded = false;
    await render(); expect(host.querySelector('[aria-label="Get started"]')).toBeNull();
    expect(fixture.persist).not.toHaveBeenCalled();
  });
  it('preserves a returning user’s customized cards without saving a replacement', async () => {
    fixture.layout!.cards[0].size.w = 4;
    const before = JSON.stringify(fixture.layout);
    await render();
    expect(host.querySelector('[data-testid="dashboard-cell-decisions"]')).toBeTruthy();
    expect(JSON.stringify(fixture.layout)).toBe(before);
    expect(fixture.persist).not.toHaveBeenCalled();
  });
  it('recognizes only shipped layouts, including the daemon’s four-card default', () => {
    expect(isDefaultHomeLayout(DEFAULT_LAYOUT)).toBe(true);
    const card = (type: string, x: number, y: number, w: number, h: number) => ({ id: type, type, position: { x, y }, size: { w, h }, visible: true });
    expect(isDefaultHomeLayout({ cards: [card('hero', 0, 0, 7, 4), card('stats', 7, 0, 5, 4), card('in_flight', 0, 4, 12, 3), card('recent', 0, 7, 12, 4)] })).toBe(true);
    const custom = structuredClone(DEFAULT_LAYOUT); custom.cards.reverse();
    expect(isDefaultHomeLayout(custom)).toBe(false);
  });
  for (const [width, height] of [[1280, 800], [1024, 640]]) for (const theme of ['dark', 'silver'] as const) for (const populated of [false, true]) for (const dockOpen of [false, true]) {
    it(`DOM Home ${populated ? 'returning' : 'untouched'} ${width}x${height} ${theme} dock-${dockOpen ? 'open' : 'closed'}`, async () => {
      Object.defineProperties(window, { innerWidth: { configurable: true, value: width }, innerHeight: { configurable: true, value: height } });
      host.style.width = `${width - 240 - (dockOpen ? (width < 1200 ? 320 : 384) : 0)}px`; host.style.height = `${height - 28}px`;
      host.dataset.viewport = `${width}x${height}`; host.dataset.dock = dockOpen ? 'open' : 'closed';
      useCommandCenter.setState({ chatDockOpen: dockOpen });
      act(() => setTheme(theme));
      if (populated) { fixture.sessions = 3; fixture.memories = 5; useCommandCenter.setState({ currentModel: 'selected-model' }); }
      await render(); expect(host).toMatchSnapshot();
    });
  }
});
