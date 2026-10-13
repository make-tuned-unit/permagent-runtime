import { sceneChildren, settleStartup } from './forumStartupTestUtils';
/** @vitest-environment jsdom */
import React, { act, Suspense } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const gpu = vi.hoisted(() => ({ fail: false, props: null as Record<string, any> | null }));
vi.mock('@react-three/fiber', () => ({ Canvas: (props: Record<string, any>) => { gpu.props = props; if (gpu.fail) throw new Error('WebGL unavailable'); return <canvas />; } }));
vi.mock('./HeroAgentPortrait', () => ({ HeroAgentPortrait: () => null }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: true }) }));
import { ForumView } from './ForumView';
import { ForumStageReady, WORLD_ASSET_DEADLINE_MS, WORLD_READY_BUDGET_MS } from './ForumStageStatus';
import { ForumPostProcessing } from './ForumPostProcessing';
import { guardForumAssetRequest, noteForumAssetLoaded, noteForumAssetRequest, resetForumAssetRetryForTests } from './forumAssetRetry';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
beforeEach(() => { resetForumAssetRetryForTests(); vi.useFakeTimers(); window.history.replaceState({}, '', '/?campus=1'); localStorage.clear(); gpu.fail = false; vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false }))); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); window.history.replaceState({}, '', '/'); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
it('shows immediate stage feedback while suspended and keeps HUD and navigation interactive', () => {
  const navigate = vi.fn(); const start = performance.now();
  act(() => root.render(<ForumView onNavigate={navigate} />));
  expect(host.textContent).toContain('Loading World…');
  const feedbackMs = performance.now() - start;
  expect(feedbackMs).toBeLessThan(500);
  const build = [...host.querySelectorAll('button')].find(b => b.textContent?.includes('Build') && b.getAttribute('role') === 'radio')!;
  act(() => build.click());
  const action = [...host.querySelectorAll('button')].find(b => b.textContent?.includes('Open Build'))!;
  expect(action).toBeTruthy(); act(() => action.click()); expect(navigate).toHaveBeenCalledWith('build');
  settleStartup();
  const ready = sceneChildren(gpu.props!).find((child: any) => child?.type === ForumStageReady);
  act(() => ready.props.onReady());
  expect(host.textContent).not.toContain('Loading World…');
  console.info('L2 loading feedback ms', feedbackMs);
});
it('offers recovery at five seconds and can still finish a slow load', () => {
  act(() => root.render(<ForumView />));
  act(() => vi.advanceTimersByTime(WORLD_READY_BUDGET_MS));
  expect(host.textContent).toContain('World is taking longer');
  expect([...host.querySelectorAll('button')].some(b => b.textContent === 'Retry World')).toBe(true);
  settleStartup();
  const ready = sceneChildren(gpu.props!).find((child: any) => child?.type === ForumStageReady);
  act(() => ready.props.onReady());
  expect(host.textContent).not.toContain('World is taking longer');
});
it('keeps recovery outside a failed Canvas and retries only on request', () => {
  vi.spyOn(console, 'error').mockImplementation(() => {}); gpu.fail = true;
  act(() => root.render(<ForumView />));
  settleStartup();
  expect(host.textContent).toContain('World could not start');
  expect(host.querySelector('[aria-label="World places"]')).toBeTruthy();
  gpu.fail = false;
  act(() => [...host.querySelectorAll('button')].find(b => b.textContent === 'Retry World')!.click());
  expect(host.querySelector('canvas')).toBeTruthy();
  expect(host.textContent).toContain('Loading World…');
});
it('keeps every scene suspension inside the Canvas so the route Suspense never hides the HUD or Retry', () => {
  // R3F v8 rethrows an uncaught in-Canvas suspension to the DOM tree; the nearest boundary there is
  // WorkspaceRenderer's World route, whose fallback replaced the whole World (native r35, stalled assets).
  act(() => root.render(<ForumView />));
  settleStartup();
  const top = React.Children.toArray(gpu.props!.children) as any[];
  const boundary = top.filter(child => child?.type === Suspense);
  expect(boundary).toHaveLength(1);
  // Outside the boundary: only components that never suspend.
  expect(top.filter(child => child?.type !== Suspense).map(child => child.type.name).sort()).toEqual(['ForumCanvasActivity', 'PerfSampler']);
  const scene = sceneChildren(boundary[0].props);
  for (const type of [ForumStageReady, ForumPostProcessing]) expect(scene.some(child => child?.type === type)).toBe(true);
  expect(scene.some(child => child?.type === 'group')).toBe(true);
});
it('Retry World drops cached failed loads and re-reads the kit manifest', () => {
  vi.spyOn(console, 'error').mockImplementation(() => {}); gpu.fail = true;
  act(() => root.render(<ForumView />));
  settleStartup();
  const failedClear = vi.fn(), loadedClear = vi.fn();
  noteForumAssetRequest('test:failed', ['world/failed.webp'], failedClear);
  noteForumAssetRequest('test:loaded', ['world/loaded.webp'], loadedClear);
  noteForumAssetLoaded('world/loaded.webp');
  const manifestReads = () => vi.mocked(fetch).mock.calls.filter(([url]) => String(url).endsWith('garden-hero/manifest.json')).length;
  const before = manifestReads();
  gpu.fail = false;
  act(() => [...host.querySelectorAll('button')].find(b => b.textContent === 'Retry World')!.click());
  expect(failedClear).toHaveBeenCalledTimes(1);
  expect(loadedClear).not.toHaveBeenCalled();
  expect(manifestReads()).toBe(before + 1);
  expect(host.textContent).toContain('Loading World…');
});
it('does not tell a failed World that it is still loading', () => {
  vi.spyOn(console, 'error').mockImplementation(() => {}); gpu.fail = true;
  act(() => root.render(<ForumView />));
  settleStartup();
  expect(host.textContent).toContain('World could not start');
  expect(host.textContent).not.toContain('while it loads');
});

it('a World asset request that never completes is reported as still loading, then as failed with Retry, and Retry refetches', () => {
  act(() => root.render(<ForumView />));
  settleStartup();
  const ready = sceneChildren(gpu.props!).find((child: any) => child?.type === ForumStageReady);
  act(() => ready.props.onReady());
  const onError = vi.fn(), clear = vi.fn();
  noteForumAssetRequest('test:hung', ['world/hung.glb'], clear);
  act(() => { guardForumAssetRequest('world/hung.glb', undefined, onError); });
  expect(host.textContent).not.toContain('scenery');
  act(() => vi.advanceTimersByTime(WORLD_READY_BUDGET_MS));
  expect(host.textContent).toContain('Still loading the World\u2019s scenery');
  expect(onError).not.toHaveBeenCalled();
  act(() => vi.advanceTimersByTime(WORLD_ASSET_DEADLINE_MS - WORLD_READY_BUDGET_MS));
  expect(onError).toHaveBeenCalledTimes(1);
  expect(host.textContent).toContain('The World\u2019s scenery did not load');
  expect(host.textContent).not.toContain('Still loading the World');
  act(() => [...host.querySelectorAll('button')].filter(b => b.textContent === 'Retry World').pop()!.click());
  expect(clear).toHaveBeenCalledTimes(1);
  expect(host.textContent).not.toContain('scenery did not load');
});
