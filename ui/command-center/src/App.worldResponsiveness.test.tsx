import { settleStartup } from './components/world/forum/forumStartupTestUtils';
/** @vitest-environment jsdom */
// Real App -> WorkspaceRenderer -> ForumView -> HabitatScene. Native/GPU and
// artwork doubles cannot measure WebKit compositor or GPU frame duration.
import { afterEach, expect, it, vi } from 'vitest';
import { act, Children, isValidElement, Profiler, useEffect, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
const trace = vi.hoisted(() => ({
  marks: [] as { name: string; at: number }[], frames: 'never', mounts: 0, disposals: 0,
  mark(name: string) { this.marks.push({ name, at: performance.now() }); },
}));
vi.mock('./components/sidebar/Sidebar', () => ({ Sidebar: () => null }));
vi.mock('./components/settings/SettingsView', () => ({ SettingsView: () => null }));
vi.mock('./components/skills/SkillsPanel', () => ({ SkillsPanel: () => null }));
vi.mock('./components/workspaces/WorkspaceSaveErrorChip', () => ({ WorkspaceSaveErrorChip: () => null }));
vi.mock('./components/wizard/WizardShell', () => ({ WizardShell: () => null }));
vi.mock('./components/splash/Splash', () => ({ Splash: () => null }));
vi.mock('./components/chat/ChatLauncher', () => ({ ChatLauncher: () => null, CHAT_LAUNCHER_MARGIN: 12 }));
vi.mock('./components/chat/ChatDock', () => ({ ChatDock: () => null }));
vi.mock('./components/voice/VoiceHost', () => ({ VoiceHost: () => null }));
vi.mock('./components/goals/GoalDetailModal', () => ({ GoalDetailModalHost: () => null }));
vi.mock('./components/projects/PersonDetailModal', () => ({ PersonDetailModalHost: () => null }));
vi.mock('./components/chat/DropZone', () => ({ DropZone: ({ children }: { children: unknown }) => children }));
vi.mock('./components/notifications/NotificationHost', () => ({ NotificationHost: () => null }));
vi.mock('./components/version/VersionSkewBanner', () => ({ VersionSkewBanner: () => null }));
vi.mock('./hooks/useAppNavigate', () => ({ useAppNavigate: () => undefined }));
vi.mock('./hooks/useVersionSkew', () => ({ useVersionSkew: () => ({}) }));
vi.mock('./lib/repaintOnRegain', () => ({ onRepaintRegain: () => () => {}, forceCompositorRepaint: () => {} }));
vi.mock('./components/history/HistoryView', () => ({ HistoryView: () => null, isHistoryTab: () => false }));
vi.mock('./components/chat/ChatView', () => ({ ChatView: () => null }));
vi.mock('./components/terminal/TerminalManager', () => ({ TerminalManager: () => null }));
vi.mock('./components/browser', () => ({ Browser: () => null }));
vi.mock('./components/trace/ExecutionTrace', () => ({ ExecutionTrace: () => null }));
vi.mock('./components/brain/BrainView', () => ({ BrainView: () => null }));
vi.mock('./components/dashboard/Dashboard', () => ({ Dashboard: () => <p>Home ready</p> }));
vi.mock('./components/build/BuildView', () => ({ BuildView: () => <p>Build ready</p> }));
vi.mock('./components/grow/GrowView', () => ({ GrowView: () => null }));
vi.mock('./components/finance/FinanceView', () => ({ FinanceView: () => null }));
vi.mock('./components/automate/AutomateView', () => ({ AutomateView: () => null }));
vi.mock('./components/projects/ProjectsView', () => ({ ProjectsView: () => null }));
vi.mock('./components/people/PeopleView', () => ({ PeopleView: () => null }));
vi.mock('./components/world/forum/HeroAgentPortrait', () => ({ HeroAgentPortrait: () => null }));
vi.mock('./components/world/forum/ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./components/world/forum/ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('./components/world/agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: true }) }));
vi.mock('./components/world/forum/HabitatArchitecture', () => ({ HabitatArchitecture: () => null }));
vi.mock('./components/world/forum/HabitatPlanting', () => ({ HabitatPlanting: () => null }));
vi.mock('./components/world/forum/HabitatWater', () => ({ HabitatWater: () => null }));
vi.mock('./components/workspaces/WorkspaceRenderer', async original => {
  const { WorkspaceRenderer } = await original<typeof import('./components/workspaces/WorkspaceRenderer')>();
  return { WorkspaceRenderer: (props: Parameters<typeof WorkspaceRenderer>[0]) => <Profiler id="WorkspaceRenderer" onRender={() => trace.mark('WorkspaceRenderer commit')}><WorkspaceRenderer {...props} /></Profiler> };
});
vi.mock('./components/world/forum/ForumView', async original => {
  const { ForumView } = await original<typeof import('./components/world/forum/ForumView')>();
  return { ForumView: (props: Parameters<typeof ForumView>[0]) => <Profiler id="ForumView" onRender={() => trace.mark('ForumView commit')}><ForumView {...props} /></Profiler> };
});
vi.mock('./components/world/forum/HabitatScene', async original => {
  const mod = await original<typeof import('./components/world/forum/HabitatScene')>();
  return { ...mod, HabitatScene: (props: Parameters<typeof mod.HabitatScene>[0]) => {
    useEffect(() => { trace.mounts++; trace.mark('HabitatScene init'); return () => { trace.disposals++; trace.mark('HabitatScene dispose'); }; }, []);
    return <Profiler id="HabitatScene" onRender={() => trace.mark('HabitatScene commit')}><mod.HabitatScene {...props} /></Profiler>;
  } };
});
vi.mock('@react-three/fiber', () => ({
  useFrame: () => {}, useThree: () => ({}),
  Canvas: ({ children, frameloop }: { children: ReactNode; frameloop: string }) => {
    trace.frames = frameloop;
    const pick = (nodes: ReactNode): ReactNode => Children.map(nodes, node => {
      if (!isValidElement<{ children?: ReactNode }>(node)) return null;
      if (typeof node.type === 'function' && node.type.name === 'HabitatScene') return node;
      return node.props.children ? pick(node.props.children) : null;
    });
    return <div data-world-loop={frameloop}>{pick(children)}</div>;
  },
}));
import { ForumAppView } from './components/world/forum/ForumAppView';
void ForumAppView; // Settle the module graph before timing navigation.
import { MainContent } from './App';
import { useCommandCenter, type WorkspaceState } from './lib/store';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const observers = new Set<ResizeObserverCallback>();
let root: ReturnType<typeof createRoot> | undefined;
let host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); host?.remove(); observers.clear(); vi.unstubAllGlobals(); });
function resize(width: number) {
  act(() => { for (const callback of observers) { trace.mark(`ResizeObserver ${width}`); callback([{ contentRect: { width, height: width } } as ResizeObserverEntry], {} as ResizeObserver); } });
}
it('traces twenty mixed switches and immediate follow-up actions across the real World route', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false })));
  vi.stubGlobal('ResizeObserver', class { constructor(cb: ResizeObserverCallback) { this.cb = cb; } cb: ResizeObserverCallback; observe() { observers.add(this.cb); } disconnect() { observers.delete(this.cb); } });
  localStorage.clear(); trace.marks = []; trace.mounts = 0; trace.disposals = 0;
  useCommandCenter.setState({ workspacesLoaded: true, activePanel: 'chat', activeWorkspaceId: 'home', workspaces: [
    { id: 'home', name: 'Home', layoutJson: { type: 'panel', tool: 'dashboard' } },
    { id: 'world', name: 'World', layoutJson: { type: 'panel', tool: 'world' } },
    { id: 'build', name: 'Build', layoutJson: { type: 'panel', tool: 'build' } },
  ] as WorkspaceState[] });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<Profiler id="App" onRender={() => trace.mark('App commit')}><MainContent /></Profiler>));
  expect(host.querySelector('.forum-shell')).toBeNull();
  const hiddenStartupMounts = trace.mounts;
  const latencies: number[] = []; let hiddenAlways = 0;
  for (let i = 0; i < 20; i++) {
    await act(async () => useCommandCenter.setState({ activeWorkspaceId: 'world', activePanel: 'chat' }));
    for (let n = 0; n < 30 && !host.querySelector('.forum-shell'); n++) await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
    expect(host.querySelector('.forum-shell')).toBeTruthy();
    resize(900);
    if(i===0)await settleStartup();
    const started = performance.now(); trace.mark('leave input');
    act(() => useCommandCenter.setState(i % 2 ? { activePanel: 'settings' } : { activeWorkspaceId: 'home' }));
    if (trace.frames === 'always') hiddenAlways++;
    act(() => useCommandCenter.setState({ activePanel: 'chat', activeWorkspaceId: 'build' }));
    expect(useCommandCenter.getState().activeWorkspaceId).toBe('build');
    expect(host.textContent).toContain('Build ready');
    trace.mark('follow-up feedback'); latencies.push(performance.now() - started);
    resize(0);
  }
  const sorted = [...latencies].sort((a,b) => a-b);
  console.info('L2 switch trace', JSON.stringify({ hiddenStartupMounts, hiddenAlways, mounts: trace.mounts, disposals: trace.disposals, p95Ms: sorted[18], maxMs: sorted[19], marks: trace.marks }));
  expect(hiddenStartupMounts).toBe(0);
  expect(hiddenAlways).toBe(0);
  expect(trace.mounts).toBe(1);
  expect(trace.disposals).toBe(0);
  expect(sorted[18]).toBeLessThan(200);
  expect(sorted[19]).toBeLessThan(500);
});
