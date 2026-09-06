/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { readFileSync } from 'node:fs';
import { BlenderVault, VAULT_ASSET } from './BlenderVault';

vi.mock('@react-three/drei', () => ({ useGLTF: vi.fn() }));
import { useGLTF } from '@react-three/drei';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => { vi.restoreAllMocks(); });

describe('Blender World asset gate', () => {
  it('ships a self-contained, bounded GLB rather than a missing remote dependency', () => {
    expect(VAULT_ASSET).toBe(`${import.meta.env.BASE_URL}world/observatory-vault.glb`);
    const bytes = readFileSync('public/world/observatory-vault.glb');
    expect(bytes.toString('utf8', 0, 4)).toBe('glTF');
    expect(bytes.readUInt32LE(4)).toBe(2);
    expect(bytes.readUInt32LE(8)).toBe(bytes.length);
    expect(bytes.length).toBeLessThan(8_000_000);
    const jsonLength = bytes.readUInt32LE(12);
    const asset = JSON.parse(bytes.toString('utf8', 20, 20 + jsonLength));
    expect(asset.meshes).toHaveLength(6);
    expect(asset.materials).toHaveLength(6);
    expect(asset.materials.some((m: { name: string }) => m.name.includes('jade foliage'))).toBe(true);
    expect(asset.buffers.every((b: { uri?: string }) => !b.uri)).toBe(true);
    expect(asset.images ?? []).toHaveLength(0);
    expect(asset.cameras ?? []).toHaveLength(0);
    const triangles = asset.meshes.flatMap((m: { primitives: { indices: number }[] }) => m.primitives)
      .reduce((sum: number, p: { indices: number }) => sum + asset.accessors[p.indices].count / 3, 0);
    expect(triangles).toBeGreaterThan(1000);
    expect(triangles).toBeLessThan(150_000);
  });

  it.each(['loading', 'failed'])('preserves the usable old hall when asset is %s', (status) => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    vi.mocked(useGLTF).mockImplementation(() => {
      if (status === 'failed') throw new Error('missing asset');
      throw new Promise(() => {});
    });
    const host = document.createElement('div');
    const root = createRoot(host);
    act(() => root.render(<BlenderVault fallback={<span>original usable hall</span>} />));
    expect(host.textContent).toBe('original usable hall');
    act(() => root.unmount());
  });
});
