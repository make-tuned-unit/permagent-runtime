import { settleStartup } from './forumStartupTestUtils';
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
      await act(async () => probe.frame());
      if(appearance !== 'day' || probe.builds === 0)await settleStartup();
      act(() => probe.frame()); times.push(performance.now() - start);
    }
    console.info('L2 environment trace', JSON.stringify({ builds: probe.builds, disposedBeforeUnmount: probe.disposed, times }));
    expect(probe.builds).toBe(2); // Cached day is assigned without another PMREM build.
    expect(probe.disposed).toBe(0);
    expect(times[2]).toBeLessThan(500);
  } finally { act(() => root.unmount()); }
  expect(probe.disposed).toBe(probe.builds);
});

it('waits for the first frame, yields before capture, and cancels hidden preparation', async () => {
  vi.useFakeTimers();probe.builds=0;probe.disposed=0;
  const applied=vi.fn(),host=document.createElement('div'),root=createRoot(host);
  try {
    await act(async()=>root.render(<ForumEnvironment appearance="day" onApplied={applied}/>));
    expect(probe.builds).toBe(0);expect(vi.getTimerCount()).toBe(0);
    await act(async()=>probe.frame());
    act(()=>vi.advanceTimersByTime(20));expect(probe.builds).toBe(0);
    act(()=>vi.advanceTimersByTime(20));expect(probe.builds).toBe(1);expect(applied).not.toHaveBeenCalled();
    act(()=>probe.frame());expect(applied).toHaveBeenCalledWith('day');
    await act(async()=>root.render(<ForumEnvironment appearance="night" active={false}/>));
    expect(vi.getTimerCount()).toBe(0);
    await act(async()=>root.render(<ForumEnvironment appearance="night"/>));
    act(()=>root.render(null));expect(vi.getTimerCount()).toBe(0);
    expect(probe.disposed).toBe(1);
  }finally{act(()=>root.unmount());vi.useRealTimers();}
});
