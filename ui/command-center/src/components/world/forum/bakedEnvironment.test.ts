import { describe, expect, it } from 'vitest';
import {
  CUBE_FACE_ORDER,
  environmentIntensity,
  environmentPath,
  faceUrls,
  loadEnvironmentManifest,
} from './bakedEnvironment';

const complete = {
  schema: 'permagent.forum.environment.v1',
  appearance: 'day' as const,
  faceSize: 512,
  faces: [...CUBE_FACE_ORDER],
};

function fetcherReturning(body: unknown, ok = true): typeof fetch {
  return (async () => ({ ok, json: async () => body })) as unknown as typeof fetch;
}

describe('the Unreal-baked environment', () => {
  it('loads a complete manifest', async () => {
    const manifest = await loadEnvironmentManifest('/ui/', 'day', fetcherReturning(complete));
    expect(manifest).toEqual(complete);
  });

  it('treats a build with no bake as ordinary, not as an error', async () => {
    const missing = await loadEnvironmentManifest('/ui/', 'day', fetcherReturning(null, false));
    expect(missing).toBeNull();
  });

  it('refuses a half-finished bake rather than lighting the world through a hole', async () => {
    // Five faces is what an interrupted capture leaves behind. A cube map built
    // from it throws inside three; better to fall back to the derived sky.
    const partial = { ...complete, faces: ['px', 'nx', 'py', 'ny', 'pz'] };
    expect(await loadEnvironmentManifest('/ui/', 'day', fetcherReturning(partial))).toBeNull();
  });

  it('refuses a manifest for the wrong appearance', async () => {
    const night = { ...complete, appearance: 'night' as const };
    expect(await loadEnvironmentManifest('/ui/', 'day', fetcherReturning(night))).toBeNull();
  });

  it('refuses an unknown schema, so a future format cannot be misread', async () => {
    const future = { ...complete, schema: 'permagent.forum.environment.v2' };
    expect(await loadEnvironmentManifest('/ui/', 'day', fetcherReturning(future))).toBeNull();
  });

  it('survives a network or parse failure', async () => {
    const throwing = (async () => {
      throw new Error('offline');
    }) as unknown as typeof fetch;
    expect(await loadEnvironmentManifest('/ui/', 'day', throwing)).toBeNull();
  });

  it('builds the six face urls in the order the cube loader expects', () => {
    expect(faceUrls('/ui/', 'night')).toEqual([
      '/ui/world/env/night/px.png',
      '/ui/world/env/night/nx.png',
      '/ui/world/env/night/py.png',
      '/ui/world/env/night/ny.png',
      '/ui/world/env/night/pz.png',
      '/ui/world/env/night/nz.png',
    ]);
    expect(environmentPath('/ui/', 'day')).toBe('/ui/world/env/day/');
  });

  it('lights night more gently than day', () => {
    expect(environmentIntensity('night')).toBeLessThan(environmentIntensity('day'));
    expect(environmentIntensity('day')).toBeLessThanOrEqual(1);
  });
});
