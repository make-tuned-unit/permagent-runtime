/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { BackSide, Color, PerspectiveCamera, Raycaster, Sphere, Vector2, Vector3 } from 'three';
import { afterEach, expect, it, vi } from 'vitest';
import { HABITAT_ATMOSPHERE, HABITAT_ATMOSPHERE_RADIUS, HabitatAtmosphere, HabitatSky, habitatAtmosphereFragment } from './HabitatSky';
import { habitatOverviewCamera } from './habitatLayout';
import { setReduceMotion } from '../../../styles/tokens';

vi.mock('@react-three/fiber', () => ({ useFrame: vi.fn(), useThree: () => ({ camera: new PerspectiveCamera() }) }));
// A pending sky texture must not suspend the independent atmosphere layer.
vi.mock('./useForumTexture', () => ({ useForumTexture: () => { throw new Promise(() => {}); } }));
import { useFrame } from '@react-three/fiber';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => { setReduceMotion(false); vi.restoreAllMocks(); vi.clearAllMocks(); });

it.each([1.6, 1.3, .9, .6, .25])('covers every overview viewport ray inside the clipping range at aspect %s', aspect => {
  const framing = habitatOverviewCamera(aspect);
  const camera = new PerspectiveCamera(framing.fov, aspect, .25, 6000);
  camera.position.set(...framing.position); camera.lookAt(new Vector3(...framing.target)); camera.updateMatrixWorld();
  const sphere = new Sphere(new Vector3(), HABITAT_ATMOSPHERE_RADIUS), raycaster = new Raycaster();
  for (const x of [-1, -.5, 0, .5, 1]) for (const y of [-1, -.5, 0, .5, 1]) {
    raycaster.setFromCamera(new Vector2(x, y), camera);
    const hit = raycaster.ray.intersectSphere(sphere, new Vector3());
    expect(hit, `missing backdrop at ${x}, ${y}`).not.toBeNull();
    const projected = hit!.clone().project(camera);
    expect(projected.z).toBeGreaterThan(-1); expect(projected.z).toBeLessThan(1);
    // The lower half that was a black void reaches the fully covered part.
    if (y <= 0) expect(hit!.normalize().y).toBeLessThan(-.22);
  }
});

it.each(['day', 'night'] as const)('keeps %s atmosphere present during suspended asset loading with no motion work', appearance => {
  const host = document.createElement('div'), root = createRoot(host);
  vi.spyOn(console, 'error').mockImplementation(() => {}); // R3F intrinsics in a DOM harness.
  try {
    for (const reduced of [false, true]) {
      setReduceMotion(reduced);
      act(() => root.render(<HabitatSky appearance={appearance}/>));
      expect([...host.querySelectorAll('sphereGeometry')].some(g => g.getAttribute('args')?.startsWith(`${HABITAT_ATMOSPHERE_RADIUS},`))).toBe(true);
      const haze = [...host.querySelectorAll('shaderMaterial')].find(m => m.getAttribute('fragmentShader') === habitatAtmosphereFragment)!;
      expect(haze).toBeTruthy(); expect(haze.getAttribute('side')).toBe(String(BackSide));
      expect(habitatAtmosphereFragment).not.toMatch(/time|noise|texture/);
    }
    expect(useFrame).not.toHaveBeenCalled();
    // Both gradient endpoints remain perceptibly above black, including nadir.
    for (const hex of Object.values(HABITAT_ATMOSPHERE[appearance])) {
      const color = new Color(hex);
      expect(Math.min(color.r, color.g, color.b)).toBeGreaterThan(.008);
    }
  } finally { act(() => root.unmount()); }
});

it('changes the atmosphere with applied day/night without replacing its sphere', () => {
  const host = document.createElement('div'), root = createRoot(host);
  vi.spyOn(console, 'error').mockImplementation(() => {});
  try {
    act(() => root.render(<HabitatAtmosphere appearance="day"/>));
    const sphere = host.querySelector('sphereGeometry');
    act(() => root.render(<HabitatAtmosphere appearance="night"/>));
    expect(host.querySelector('sphereGeometry')).toBe(sphere);
    expect(host.querySelectorAll('mesh')).toHaveLength(1);
  } finally { act(() => root.unmount()); }
});
