/** @vitest-environment jsdom */
import { Component, Suspense, act, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useLoader } from '@react-three/fiber';
import { WORLD_ASSET_DEADLINE_MS } from './ForumStageStatus';
import { clearUnsettledForumAssets, ForumAssetTimeoutError, forumAssetManager, resetForumAssetRetryForTests, useForumAssetHealth } from './forumAssetRetry';
import { extendForumLoader } from './forumGltf';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// Native r36 run A3: a model request that never completes left the World "ready" with no agents and no
// notice, and Retry could not refetch because the hung request stayed in the loader caches.
class Boundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() { return this.state.failed ? <p>failed</p> : this.props.children; }
}
let root: Root, host: HTMLDivElement, attempt = 0;
const mount = async (node: ReactNode) => { attempt++; await act(async () => root.render(<Boundary key={attempt}><Suspense fallback={<p>loading</p>}>{node}</Suspense></Boundary>)); };
beforeEach(() => { resetForumAssetRetryForTests(); vi.useFakeTimers(); vi.spyOn(console, 'error').mockImplementation(() => {}); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); vi.restoreAllMocks(); });

const net = { hang: true, requests: [] as string[] };
class HangingGltfLoader {
  setMeshoptDecoder() { return this; }
  load(url: string, onLoad: (data: unknown) => void) {
    net.requests.push(url);
    if (!net.hang) setTimeout(() => onLoad({ asset: url }), 0);
  }
}
function Glb({ url }: { url: string }) {
  const data = useLoader(HangingGltfLoader as never, url, loader => extendForumLoader(loader as never)) as unknown as { asset: string };
  return <p>loaded {data.asset}</p>;
}
function Health() { const h = useForumAssetHealth(); return <p>health {h.pending}/{h.timedOut}</p>; }

it('a never-resolving GLB fails at the deadline, and Retry starts a fresh request', async () => {
  const url = '/world/test-hung.glb'; net.hang = true; net.requests = [];
  const abort = vi.spyOn(forumAssetManager, 'abort');
  await mount(<><Health /><Glb url={url} /></>);
  expect(host.textContent).toContain('loading');
  await act(async () => { vi.advanceTimersByTime(WORLD_ASSET_DEADLINE_MS - 1); });
  expect(host.textContent).toContain('loading');           // still waiting, not yet failed
  await act(async () => { vi.advanceTimersByTime(1); });
  await act(async () => { await Promise.resolve(); });
  expect(host.textContent).toBe('failed');                 // bounded failure reaches the boundary
  await mount(<Glb url={url} />);
  expect(host.textContent).toBe('failed');                 // cached rejection, no new request yet
  expect(net.requests).toEqual([url]);
  net.hang = false;
  expect(clearUnsettledForumAssets()).toBe(1);
  expect(abort).toHaveBeenCalled();
  await mount(<Glb url={url} />);
  await act(async () => { vi.advanceTimersByTime(10); });
  expect(host.textContent).toBe(`loaded ${url}`);
  expect(net.requests).toEqual([url, url]);                // Retry made a second request
});

it('reports pending then timed-out health, and ignores a result that arrives after the deadline', async () => {
  const { guardForumAssetRequest } = await import('./forumAssetRetry');
  await mount(<Health />);
  const onLoad = vi.fn(), onError = vi.fn();
  let guarded!: ReturnType<typeof guardForumAssetRequest<string>>;
  await act(async () => { guarded = guardForumAssetRequest<string>('/world/late.webp', onLoad, onError); });
  expect(host.textContent).toBe('health 1/0');
  await act(async () => { vi.advanceTimersByTime(WORLD_ASSET_DEADLINE_MS); });
  expect(host.textContent).toBe('health 0/1');
  expect(onError.mock.calls[0][0]).toBeInstanceOf(ForumAssetTimeoutError);
  guarded.onLoad('late');
  expect(onLoad).not.toHaveBeenCalled();
  await act(async () => { clearUnsettledForumAssets(); });
  expect(host.textContent).toBe('health 0/0');
});
