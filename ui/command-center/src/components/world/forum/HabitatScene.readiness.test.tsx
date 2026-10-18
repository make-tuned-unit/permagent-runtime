/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const { useForumBuild } = vi.hoisted(() => ({ useForumBuild: vi.fn() }));

vi.mock('./useForumBuild', () => ({ useForumBuild }));
vi.mock('./forumStartup', () => ({
  loadForumModule: (_name: string, load: () => Promise<unknown>) => load(),
  measureForumStartup: (_name: string, run: () => unknown) => run(),
}));
// These detail layers intentionally never report readiness. The test exercises
// the real HabitatScene component and its terrain build hook, not their assets.
vi.mock('./HabitatArchitecture', () => ({ HabitatArchitecture: () => null }));
vi.mock('./HabitatPlanting', () => ({ HabitatPlanting: () => null }));
vi.mock('./HabitatWater', () => ({ HabitatWater: () => null }));

import { HabitatScene } from './HabitatScene';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
const coreTerrain = { ground: null, dispose: vi.fn() };
const onReady = vi.fn();

beforeEach(() => {
  useForumBuild.mockReset().mockReturnValue(null);
  onReady.mockReset();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

function render() {
  root.render(<HabitatScene directing={null} onDirected={vi.fn()} onReady={onReady} />);
}

it('signals only after core terrain/collision is built, without waiting for detail layers', async () => {
  await act(async () => { render(); });
  expect(useForumBuild).toHaveBeenCalledWith(
    'HabitatScene terrain / collision', expect.any(Function), [], expect.any(Function),
  );
  expect(onReady).not.toHaveBeenCalled();

  // Complete just the real scene's core builder. Architecture and planting
  // components remain mounted but pending and never invoke an onReady callback.
  useForumBuild.mockReturnValue(coreTerrain);
  await act(async () => { render(); });
  expect(onReady).toHaveBeenCalledTimes(1);
});
