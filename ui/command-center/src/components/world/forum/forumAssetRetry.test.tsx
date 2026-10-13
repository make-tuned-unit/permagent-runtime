/** @vitest-environment jsdom */
import { Component, Suspense, act, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const gpu = vi.hoisted(() => ({ renderer: { initTexture: () => {} } }));
vi.mock('@react-three/fiber', async original => ({ ...(await original<typeof import('@react-three/fiber')>()), useThree: (pick: (state: { gl: unknown }) => unknown) => pick({ gl: gpu.renderer }) }));
import { useLoader } from '@react-three/fiber';
import { clearUnsettledForumAssets, resetForumAssetRetryForTests } from './forumAssetRetry';
import { extendForumLoader } from './forumGltf';
import { useForumTexture } from './useForumTexture';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// Native r35 (scene-recovery receipt): with scene assets failing, "Retry World" went back to
// "World could not start" in 94 ms without a single new request, because R3F's useLoader keeps
// a rejected load cached for the page's lifetime.
class Boundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() { return this.state.failed ? <p>failed</p> : this.props.children; }
}
let root: Root, host: HTMLDivElement, attempt = 0;
const mount = async (node: ReactNode) => { attempt++; await act(async () => root.render(<Boundary key={attempt}><Suspense fallback={<p>loading</p>}>{node}</Suspense></Boundary>)); };
const settle = async () => { for (let i = 0; i < 10; i++) await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); }); };
beforeEach(() => { resetForumAssetRetryForTests(); vi.spyOn(console, 'error').mockImplementation(() => {}); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

const net = { fail: true, requests: [] as string[] };
// Stands in for the shared GLTFLoader that drei's useGLTF hands to `extendForumLoader`.
class FakeGltfLoader {
  setMeshoptDecoder() { return this; }
  load(url: string, onLoad: (data: unknown) => void, _progress?: unknown, onError?: (error: unknown) => void) {
    net.requests.push(url);
    setTimeout(() => net.fail ? onError?.(new Error('blocked')) : onLoad({ asset: url }), 0);
  }
}
function Glb({ url }: { url: string }) {
  const data = useLoader(FakeGltfLoader as never, url, loader => extendForumLoader(loader as never)) as unknown as { asset: string };
  return <p>loaded {data.asset}</p>;
}

it('a failed GLB stays failed on remount until Retry clears it, then loads; loaded GLBs are kept', async () => {
  const url = '/world/test-skiff.glb'; net.fail = true; net.requests = [];
  await mount(<Glb url={url} />); await settle();
  expect(host.textContent).toBe('failed');
  await mount(<Glb url={url} />); await settle();
  expect(host.textContent).toBe('failed');           // the cache trap: a plain remount re-throws
  expect(net.requests).toEqual([url]);
  net.fail = false;
  expect(clearUnsettledForumAssets()).toBe(1);
  await mount(<Glb url={url} />); await settle();
  expect(host.textContent).toBe(`loaded ${url}`);
  expect(net.requests).toEqual([url, url]);
  expect(clearUnsettledForumAssets()).toBe(0);       // nothing loaded is thrown away
  await mount(<Glb url={url} />); await settle();
  expect(host.textContent).toBe(`loaded ${url}`);
  expect(net.requests).toEqual([url, url]);
});

function Textures({ urls }: { urls: string[] }) {
  const maps = useForumTexture(urls);
  return <p>textures {maps.length}</p>;
}
it('a failed texture set is requested again after Retry clears it', async () => {
  const urls = ['/world/test-a.webp', '/world/test-b.webp']; const requests: string[] = []; let fail = true;
  vi.stubGlobal('fetch', vi.fn(async (url: string) => { requests.push(String(url)); if (fail) throw new TypeError('blocked'); return { ok: true, status: 200, blob: async () => new Blob() }; }));
  vi.stubGlobal('createImageBitmap', vi.fn(async () => ({ width: 1, height: 1, close() {} })));
  await mount(<Textures urls={urls} />); await settle();
  expect(host.textContent).toBe('failed');
  await mount(<Textures urls={urls} />); await settle();
  expect(host.textContent).toBe('failed');
  expect(requests).toHaveLength(2);
  fail = false;
  expect(clearUnsettledForumAssets()).toBe(1);
  await mount(<Textures urls={urls} />); await settle();
  expect(host.textContent).toBe('textures 2');
  expect(requests).toHaveLength(4);
  expect(clearUnsettledForumAssets()).toBe(0);
});
