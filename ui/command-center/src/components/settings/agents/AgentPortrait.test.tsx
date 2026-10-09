/** @vitest-environment jsdom */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { AgentPortrait } from './AgentPortrait';

const thumbnails = vi.hoisted(() => ({
  get: vi.fn(async (worldId: string) => `data:image/png;base64,${worldId}`),
}));
vi.mock('./thumbnailCache', () => ({ getAgentThumbnail: thumbnails.get }));

let host: HTMLDivElement;
let root: Root;
let enterViewport: (() => void) | undefined;

beforeEach(() => {
  thumbnails.get.mockClear();
  enterViewport = undefined;
  vi.stubGlobal('WebGLRenderingContext', class {});
  vi.stubGlobal('IntersectionObserver', class {
    constructor(callback: IntersectionObserverCallback) {
      enterViewport = () => callback([{ isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
    }
    observe() {}
    disconnect() {}
  });
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});

it('loads the current World design only after a mapped portrait becomes visible', async () => {
  await act(async () => { root.render(<AgentPortrait agentId="git_steward" size={28} />); });
  expect(thumbnails.get).not.toHaveBeenCalled();
  expect(host.querySelector('[role="img"]')?.getAttribute('aria-label')).toContain('portrait unavailable');

  await act(async () => {
    enterViewport?.();
    await vi.waitFor(() => expect(thumbnails.get).toHaveBeenCalledWith('steward'));
  });
  expect(host.querySelector('img')?.getAttribute('src')).toBe('data:image/png;base64,steward');
  expect(host.querySelector('[role="img"]')?.getAttribute('aria-label')).toContain('World portrait');
});

it('keeps a truthful neutral fallback when WebGL is unavailable', async () => {
  vi.stubGlobal('WebGLRenderingContext', undefined);
  await act(async () => { root.render(<AgentPortrait agentId="forecaster" />); });
  enterViewport?.();
  expect(thumbnails.get).not.toHaveBeenCalled();
  expect(host.querySelector('img')).toBeNull();
  expect(host.querySelector('[role="img"]')?.getAttribute('aria-label')).toContain('portrait unavailable');
});
