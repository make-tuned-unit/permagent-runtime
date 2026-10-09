import { beforeEach, afterEach, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({
  contexts: 0,
  renders: [] as string[],
  disposed: 0,
  lost: 0,
  failId: '' as string,
  loads: 0,
  loadFailures: 0,
}));

vi.mock('three', async importOriginal => {
  const actual = await importOriginal<typeof import('three')>();
  return {
    ...actual,
    WebGLRenderer: class {
      domElement = { toDataURL: () => 'data:image/png;base64,example' };
      toneMapping = 0;
      outputColorSpace = '';
      constructor() { state.contexts += 1; }
      setPixelRatio() {}
      setSize() {}
      setClearColor() {}
      getContext() { return { isContextLost: () => false }; }
      render(scene: InstanceType<typeof actual.Scene>) {
        state.renders.push(scene.children.find(child => child.name)?.name ?? 'missing');
      }
      dispose() { state.disposed += 1; }
      forceContextLoss() { state.lost += 1; }
    },
  };
});

vi.mock('three/examples/jsm/loaders/GLTFLoader.js', async () => {
  const { Group } = await import('three');
  return { GLTFLoader: class {
    loadAsync() {
      state.loads += 1;
      if (state.loadFailures > 0) {
        state.loadFailures -= 1;
        return Promise.reject(new Error('temporary asset failure'));
      }
      return Promise.resolve({ scene: new Group() });
    }
  } };
});
vi.mock('../../world/forum/forumGltf', () => ({ withMeshopt: (loader: unknown) => loader }));
vi.mock('../../world/forum/heroRobot', async () => {
  const { Group } = await import('three');
  return {
    HERO_KIT_URL: '/world/garden-hero/hero-kit-v3b.glb',
    buildHeroRobot: (_scene: unknown, id: string) => {
      if (id === state.failId) throw new Error('model unavailable');
      const object = new Group();
      object.name = id;
      return { object };
    },
    disposeHeroRobot: () => {},
    heroProportions: () => [1, 1, 1],
  };
});

beforeEach(() => {
  vi.useFakeTimers();
  vi.resetModules();
  state.contexts = 0;
  state.renders = [];
  state.disposed = 0;
  state.lost = 0;
  state.failId = '';
  state.loads = 0;
  state.loadFailures = 0;
});
afterEach(() => { vi.useRealTimers(); });

it('shares duplicate requests, serializes distinct portraits on one context, and releases it idle', async () => {
  const { getAgentThumbnail } = await import('./thumbnailCache');
  const first = getAgentThumbnail('librarian');
  const duplicate = getAgentThumbnail('librarian');
  const second = getAgentThumbnail('council');
  expect(duplicate).toBe(first);
  expect(await Promise.all([first, second])).toEqual([
    'data:image/png;base64,example',
    'data:image/png;base64,example',
  ]);
  expect(state.renders).toEqual(['librarian', 'council']);
  expect(state.contexts).toBe(1);
  expect(getAgentThumbnail('librarian')).toBe(first);

  vi.advanceTimersByTime(3001);
  expect(state.disposed).toBe(1);
  expect(state.lost).toBe(1);
});

it('returns a neutral failure for one portrait without blocking the queue', async () => {
  state.failId = 'librarian';
  const { getAgentThumbnail } = await import('./thumbnailCache');
  expect(await getAgentThumbnail('librarian')).toBeNull();
  expect(await getAgentThumbnail('watcher')).toBe('data:image/png;base64,example');
  expect(state.renders).toEqual(['watcher']);
  vi.advanceTimersByTime(3001);
});

it('retries the same id after a failed hero-kit load on a later call', async () => {
  state.loadFailures = 1;
  const { getAgentThumbnail } = await import('./thumbnailCache');
  expect(await getAgentThumbnail('council')).toBeNull();
  expect(state.renders).toEqual([]);

  expect(await getAgentThumbnail('council')).toBe('data:image/png;base64,example');
  expect(state.loads).toBe(2);
  expect(state.renders).toEqual(['council']);
  vi.advanceTimersByTime(3001);
});
