import { afterEach, describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { advanceForumSurfaceClock, applyForumSurfaceMaterials, disposeForumSurfaceMaterials, releaseForumSurfaceClock } from './ForumSurfaceMaterials';

const cleanups: Array<() => void> = [];

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
});

function mesh(material: THREE.Material | THREE.Material[], name: string): THREE.Mesh {
  const result = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), material);
  result.name = name;
  return result;
}

describe('forum runtime surface materials', () => {
  it('keeps sharing within a root while adding world-space shader variation', () => {
    const source = new THREE.MeshStandardMaterial({ color: '#bcae94', roughness: 0.4 });
    source.name = 'Limestone';
    const root = new THREE.Group();
    const first = mesh(source, 'Forum terrace west');
    const second = mesh(source, 'Forum terrace east');
    root.add(first, second);

    const cleanup = applyForumSurfaceMaterials(root);
    cleanups.push(cleanup);

    expect(first.material).toBe(second.material);
    expect(first.material).not.toBe(source);
    expect((first.material as THREE.MeshStandardMaterial).roughness).toBeCloseTo(0.62);

    const material = first.material as THREE.MeshStandardMaterial;
    const shader = {
      uniforms: {},
      vertexShader: 'varying vec3 vViewPosition;\n#include <worldpos_vertex>',
      fragmentShader: 'varying vec3 vViewPosition;\n#include <color_fragment>\n#include <roughnessmap_fragment>',
    } as Parameters<THREE.MeshStandardMaterial['onBeforeCompile']>[0];
    material.onBeforeCompile(shader, undefined as never);
    expect(shader.vertexShader).toContain('forumSurfaceWorldPosition');
    expect(shader.fragmentShader).toContain('forumSurfaceValueNoise');
    expect(shader.fragmentShader).toContain('forumSurfaceWorldPosition * 1.800');
    expect(shader.fragmentShader.match(/float forumSurface(Color|Roughness)Value/g)).toHaveLength(2);
    expect(shader.fragmentShader).not.toContain('float forumSurfaceValue =');

    const once = shader.fragmentShader;
    material.onBeforeCompile(shader, undefined as never);
    expect(shader.fragmentShader).toBe(once);
    expect(material.customProgramCacheKey()).toContain('forum-surface-materials-v1:stone');
  });

  it('upgrades named standard glass to physical transmission without changing the source', () => {
    const normalMap = new THREE.DataTexture(new Uint8Array([128, 128, 255, 255]), 1, 1);
    const source = new THREE.MeshStandardMaterial({ color: '#1b5d55', roughness: 0.25, metalness: 0.4, normalMap });
    source.name = 'Jade photovoltaic glass';
    const root = new THREE.Group();
    const panel = mesh(source, 'Solar canopy');
    root.add(panel);

    const cleanup = applyForumSurfaceMaterials(root);
    cleanups.push(cleanup);

    const glass = panel.material as THREE.MeshPhysicalMaterial;
    expect(glass).not.toBe(source);
    expect(glass.isMeshPhysicalMaterial).toBe(true);
    expect(glass.defines).toMatchObject({ STANDARD: '', PHYSICAL: '' });
    expect(glass.transmission).toBeCloseTo(0.08);
    expect(glass.ior).toBeCloseTo(1.5);
    expect(glass.metalness).toBeCloseTo(0.35);
    expect(glass.normalMap).toBe(normalMap);
    expect(source.metalness).toBeCloseTo(0.4);
    expect((source as THREE.MeshStandardMaterial & { transmission?: number }).transmission).toBeUndefined();
  });

  it('leaves authored emissive inlays intact and disposes only owned clones', () => {
    const inlay = new THREE.MeshStandardMaterial({ color: '#00aaff', emissive: '#00aaff', emissiveIntensity: 1.4 });
    inlay.name = 'Engraved intelligence';
    const stone = new THREE.MeshStandardMaterial({ color: '#252535', roughness: 0.8 });
    stone.name = 'Permagent dark stone';
    const root = new THREE.Group();
    const inlayMesh = mesh(inlay, 'BUILD engraved display');
    const stoneMesh = mesh(stone, 'BUILD threshold');
    root.add(inlayMesh, stoneMesh);

    const cleanup = applyForumSurfaceMaterials(root);
    cleanups.push(cleanup);
    const ownedStone = stoneMesh.material as THREE.MeshStandardMaterial;
    let disposed = 0;
    ownedStone.addEventListener('dispose', () => disposed++);

    expect(inlayMesh.material).toBe(inlay);
    expect(stoneMesh.material).not.toBe(stone);
    disposeForumSurfaceMaterials(root);
    expect(inlayMesh.material).toBe(inlay);
    expect(stoneMesh.material).toBe(stone);
    expect(disposed).toBe(1);
    expect(() => cleanup()).not.toThrow();
  });

  it('restores material arrays and makes cleanup safe for StrictMode reapplication', () => {
    const stone = new THREE.MeshStandardMaterial({ color: '#777777' });
    stone.name = 'Stone paving';
    const timber = new THREE.MeshStandardMaterial({ color: '#6b3818' });
    timber.name = 'Oiled structural timber';
    const root = new THREE.Group();
    const slab = mesh([stone, timber], 'Forum terrace');
    root.add(slab);

    const firstCleanup = applyForumSurfaceMaterials(root);
    expect(applyForumSurfaceMaterials(root)).toBe(firstCleanup);
    firstCleanup();
    expect(slab.material).toEqual([stone, timber]);

    const secondCleanup = applyForumSurfaceMaterials(root);
    expect(secondCleanup).not.toBe(firstCleanup);
    expect(slab.material).not.toEqual([stone, timber]);
    secondCleanup();
    expect(slab.material).toEqual([stone, timber]);
  });
});

describe('the water moves', () => {
  it('gives a water material a clock, and leaves still surfaces alone', () => {
    const water = new THREE.MeshStandardMaterial({ name: 'Water' });
    const stone = new THREE.MeshStandardMaterial({ name: 'Sandstone' });
    const root = new THREE.Group();
    root.add(new THREE.Mesh(new THREE.BufferGeometry(), water));
    root.add(new THREE.Mesh(new THREE.BufferGeometry(), stone));
    const cleanup = applyForumSurfaceMaterials(root);

    const shaderFor = (mesh: THREE.Mesh) => {
      const material = mesh.material as THREE.MeshStandardMaterial;
      const shader = {
        uniforms: {} as Record<string, { value: number }>,
        vertexShader: 'varying vec3 vViewPosition;\n#include <worldpos_vertex>',
        fragmentShader:
          'varying vec3 vViewPosition;\n#include <color_fragment>\n#include <roughnessmap_fragment>\n#include <normal_fragment_maps>',
      } as Parameters<THREE.MeshStandardMaterial['onBeforeCompile']>[0];
      material.onBeforeCompile(shader, undefined as never);
      return shader;
    };

    const waterShader = shaderFor(root.children[0] as THREE.Mesh);
    expect(waterShader.fragmentShader).toContain('uniform float forumTime;');
    // The highlight has to travel, not just the tint: without a perturbed
    // normal a flat sheet of water reads as a still photograph of water.
    expect(waterShader.fragmentShader).toContain('normal = normalize(');
    expect(waterShader.uniforms.forumTime).toBeDefined();

    const stoneShader = shaderFor(root.children[1] as THREE.Mesh);
    expect(stoneShader.fragmentShader).not.toContain('forumTime');
    expect(stoneShader.uniforms.forumTime).toBeUndefined();

    // Advancing the clock reaches the compiled program by reference.
    const before = waterShader.uniforms.forumTime.value;
    advanceForumSurfaceClock(0.05);
    expect(waterShader.uniforms.forumTime.value).toBeGreaterThan(before);

    // A backgrounded tab returning after a minute must not teleport the swell.
    const beforeJump = waterShader.uniforms.forumTime.value;
    advanceForumSurfaceClock(60);
    // `toBeCloseTo`, because the clamp is a float sum: 0.1 lands as
    // 0.10000000000000002 and an exact bound would fail on arithmetic dust
    // rather than on behaviour.
    expect(waterShader.uniforms.forumTime.value - beforeJump).toBeCloseTo(0.1, 6);

    releaseForumSurfaceClock();
    cleanup();
  });
});
