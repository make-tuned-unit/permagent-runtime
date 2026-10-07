/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act, type ReactElement, type ReactNode, isValidElement } from 'react';
import { HabitatNavigation } from './HabitatScene';
import { HeroGardenAgents } from './HeroGardenKit';
import { ForumAgents } from './ForumAgents';
import { ForumWalk } from './ForumWalk';
import { HabitatAgentDock } from './HabitatAgentDock';
import { nextAgentFocus } from './agentFocus';
import { commandHabitatAgent } from './habitatMotion';
import { askFromForum } from './forumQuery';
import { useCommandCenter } from '../../../lib/store';

let canvasChildren: ReactNode = null;

vi.mock('@react-three/fiber', () => ({
  Canvas: ({ children }: { children?: ReactNode }) => {
    canvasChildren = children;
    return null;
  },
  useFrame: vi.fn(),
  useThree: vi.fn(() => ({ camera: { position: { copy: vi.fn(), clone: vi.fn() }, fov: 58, updateProjectionMatrix: vi.fn() }, gl: {} })),
}));
vi.mock('@react-three/drei', () => {
  const useGLTF = Object.assign(
    () => ({ scene: { clone: () => ({ traverse: () => {}, position: { set: () => {} }, rotation: { set: () => {} }, scale: { set: () => {} } }) } }),
    { preload: vi.fn() },
  );
  return { useGLTF, OrbitControls: () => null, Html: () => null, useTexture: () => [] };
});
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Hank' }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./forumQuery', () => ({ askFromForum: vi.fn() }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: false }) }));
vi.mock('./habitatMotion', async () => {
  const actual = await vi.importActual<typeof import('./habitatMotion')>('./habitatMotion');
  return {
    ...actual,
    commandHabitatAgent: vi.fn(() => true),
    stopHabitatAgent: vi.fn(),
  };
});

import { ForumView } from './ForumView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function findByType(node: ReactNode, type: unknown): ReactElement | null {
  if (node == null || typeof node === 'boolean') return null;
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findByType(child, type);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement(node)) return null;
  if (node.type === type) return node;
  return findByType((node.props as { children?: ReactNode }).children, type);
}

function agentSelectFromCanvas(): ((id: string) => void) | null {
  const hero = findByType(canvasChildren, HeroGardenAgents);
  if (hero) return (hero.props as { onSelect: (id: string) => void }).onSelect;
  const fallback = findByType(canvasChildren, ForumAgents);
  if (fallback) return (fallback.props as { onSelectAgent: (id: string) => void }).onSelectAgent;
  return null;
}

function navigationFocus(): { id: string; revision: number } | null | undefined {
  const nav = findByType(canvasChildren, HabitatNavigation);
  return nav ? (nav.props as { focusAgent: { id: string; revision: number } | null }).focusAgent : undefined;
}

describe('nextAgentFocus', () => {
  it('bumps revision on repeated same-agent focus', () => {
    const first = nextAgentFocus(null, 'librarian');
    expect(first).toEqual({ id: 'librarian', revision: 1 });
    expect(nextAgentFocus(first, 'librarian')).toEqual({ id: 'librarian', revision: 2 });
  });
});

describe('ForumView agent select→focus wiring', () => {
  let root: Root;
  let host: HTMLDivElement;

  beforeEach(() => {
    canvasChildren = null;
    window.history.replaceState({}, '', '/');
    vi.mocked(commandHabitatAgent).mockClear();
    vi.mocked(askFromForum).mockClear();
    useCommandCenter.setState({
      chatWindowOpen: false,
      worldConversationActive: false,
      chatDockOpen: true,
      voiceEngine: null,
      daemonAuthError: null,
    });
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  });

  async function renderWithManifest(ok: boolean) {
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo) => {
      const url = String(input);
      if (url.includes('garden-hero/manifest.json')) {
        return ok
          ? { ok: true, json: async () => ({ version: 'hero-kit-v3b' }) }
          : { ok: false, json: async () => ({}) };
      }
      return { ok: false, json: async () => ({}) };
    }));
    await act(async () => {
      root.render(<ForumView />);
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it('frames from the hero-kit onSelect callback and bumps revision on repeat', async () => {
    await renderWithManifest(true);
    const onSelect = agentSelectFromCanvas();
    expect(onSelect).toBeTypeOf('function');
    expect(findByType(canvasChildren, HeroGardenAgents)).toBeTruthy();

    await act(async () => { onSelect!('librarian'); });
    expect(navigationFocus()).toEqual({ id: 'librarian', revision: 1 });
    await act(async () => { onSelect!('librarian'); });
    expect(navigationFocus()).toEqual({ id: 'librarian', revision: 2 });
    expect(commandHabitatAgent).not.toHaveBeenCalled();
    expect(askFromForum).not.toHaveBeenCalled();
    expect(useCommandCenter.getState().voiceEngine).toBeNull();
  });

  it('exits Walk and focuses when an agent is selected', async () => {
    await renderWithManifest(true);
    await act(async () => {
      [...host.querySelectorAll('button')].find(b => b.textContent?.includes('Walk the world'))!.click();
    });
    expect(findByType(canvasChildren, ForumWalk)).toBeTruthy();
    expect(findByType(canvasChildren, HabitatNavigation)).toBeNull();

    const onSelect = agentSelectFromCanvas();
    await act(async () => { onSelect!('watcher'); });
    expect(findByType(canvasChildren, ForumWalk)).toBeNull();
    expect(navigationFocus()).toEqual({ id: 'watcher', revision: 1 });
    expect(commandHabitatAgent).not.toHaveBeenCalled();
  });

  it('uses ForumAgents onSelectAgent when the hero manifest is unavailable', async () => {
    await renderWithManifest(false);
    expect(findByType(canvasChildren, ForumAgents)).toBeTruthy();
    expect(findByType(canvasChildren, HeroGardenAgents)).toBeNull();
    const onSelect = agentSelectFromCanvas();
    await act(async () => { onSelect!('picker'); });
    expect(navigationFocus()).toEqual({ id: 'picker', revision: 1 });
    expect(commandHabitatAgent).not.toHaveBeenCalled();
  });

  it('fires dock collaborator changes through the same select path without moving', async () => {
    await renderWithManifest(true);
    expect(host.querySelector('[aria-label="Agent movement and work"]')).toBeTruthy();
    const select = host.querySelector('#habitat-agent') as HTMLSelectElement;
    await act(async () => {
      select.value = 'steward';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(navigationFocus()).toEqual({ id: 'steward', revision: 1 });
    expect(commandHabitatAgent).not.toHaveBeenCalled();
    expect(askFromForum).not.toHaveBeenCalled();
  });
});

describe('HabitatAgentDock DOM select', () => {
  let root: Root;
  let host: HTMLDivElement;
  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  it('invokes onSelect from the collaborator picker', () => {
    const onSelect = vi.fn();
    host = document.createElement('div');
    document.body.append(host);
    root = createRoot(host);
    act(() => root.render(
      <HabitatAgentDock
        selected="henry"
        name="Hank"
        directing={false}
        following={false}
        onSelect={onSelect}
        onDirect={() => {}}
        onFollow={() => {}}
        onInspect={() => {}}
        onBrief={() => {}}
        onConversation={() => {}}
      />,
    ));
    const select = host.querySelector('#habitat-agent') as HTMLSelectElement;
    act(() => {
      select.value = 'librarian';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(onSelect).toHaveBeenCalledWith('librarian');
  });
});
