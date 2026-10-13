/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const probe = vi.hoisted(() => ({ builds: 0, disposed: 0, scene: { environment: null as unknown, environmentIntensity: 1, traverse: () => {} }, gl: {}, frame: (() => {}) as () => void }));
vi.mock('@react-three/fiber', () => ({ useThree: (pick: (s: unknown) => unknown) => pick({ scene: probe.scene, gl: probe.gl }), useFrame: (cb: () => void) => { probe.frame = cb; } }));
vi.mock('./bakedEnvironment', async original => ({ ...await original<typeof import('./bakedEnvironment')>(), loadEnvironmentManifest: vi.fn(async () => null) }));
vi.mock('three', async original => ({ ...await original<typeof import('three')>(), PMREMGenerator: class {
  compileCubemapShader() {} dispose() {}
  fromScene() { probe.builds++; return { texture: {}, dispose() { probe.disposed++; } }; }
} }));
import { ForumEnvironment } from './ForumEnvironment';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
it('measures day night day environment preparation and releases owned targets', async () => {
  const host = document.createElement('div'); const root = createRoot(host); const times: number[] = [];
  try {
    for (const appearance of ['day', 'night', 'day'] as const) {
      const start = performance.now();
      await act(async () => root.render(<ForumEnvironment appearance={appearance} />));
      act(() => probe.frame()); times.push(performance.now() - start);
    }
    console.info('L2 environment trace', JSON.stringify({ builds: probe.builds, disposedBeforeUnmount: probe.disposed, times }));
    expect(probe.builds).toBe(2); // Cached day is assigned without another PMREM build.
    expect(probe.disposed).toBe(0);
    expect(times[2]).toBeLessThan(500);
  } finally { act(() => root.unmount()); }
  expect(probe.disposed).toBe(probe.builds);
});
