/** @vitest-environment jsdom */
// Job 18 bug 3: the gas giant and the moon were measurably *darker* than the
// sky they hang in — overpainted by the dome, lit from a bearing they do not
// share, and (for the gas giant) parked behind the ridge. These are the
// invariants that made that possible, pinned so a regression is a red test
// rather than another night screenshot.
import { expect, it, vi } from 'vitest';
import { createRoot } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import * as THREE from 'three';
vi.mock('@react-three/fiber', () => ({ useFrame: () => {} }));
vi.mock('@react-three/drei', () => ({ useGLTF: Object.assign(() => ({ scene: new THREE.Group() }), { preload: () => {} }) }));
import { projectPoint } from './vistaCamera';
import { FORUM_LIGHT } from './ForumLighting';
import { ForumSky, createCelestialBodies, CELESTIAL_NAMES, SKY_RENDER_ORDER, GAS_GIANT_POS, MOON_POS, GAS_GIANT_RADIUS, MOON_RADIUS } from './ForumSky';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const degrees = (radians: number) => radians * 180 / Math.PI;
/** Elevation above the horizon, in degrees. */
const elevation = (v: THREE.Vector3) => degrees(Math.asin(v.y / v.length()));
/** The Blender polar angle the landform script is authored in (glTF maps
 *  Blender (x, y, z) to three (x, z, -y)), normalised to 0-360. */
const blenderBearing = (v: THREE.Vector3) => (degrees(Math.atan2(-v.z, v.x)) + 360) % 360;
/** The ridge mesh spans exactly this arc (`scripts/blender/forum_landform.py`
 *  `_add_ridge_and_waterfalls`), crest 55-78 m at radius 128-180, i.e. up to
 *  about 26 degrees of elevation seen from the commons. */
const RIDGE_ARC = [100, 215];

it('draws the bodies after the sky dome and the starfield, and lets the world occlude them', () => {
  const { gasGiant, moon } = createCelestialBodies();
  for (const mesh of [gasGiant, moon]) {
    expect(mesh.renderOrder).toBe(SKY_RENDER_ORDER.bodies);
    expect(mesh.renderOrder).toBeGreaterThan(SKY_RENDER_ORDER.dome);
    expect(mesh.renderOrder).toBeGreaterThan(SKY_RENDER_ORDER.stars);
    // The world itself is at the default 0, so its opaque geometry still paints
    // over a body it genuinely stands in front of.
    expect(mesh.renderOrder).toBeLessThan(0);
    expect(mesh.visible).toBe(true);
    // Depth-writing and opaque: this is what keeps the transparent starfield —
    // drawn after every opaque object — from speckling through the disc.
    expect(mesh.material.depthWrite).toBe(true);
    expect(mesh.material.depthTest).toBe(true);
    expect(mesh.material.transparent).toBe(false);
  }
  expect(SKY_RENDER_ORDER.dome).toBeLessThan(SKY_RENDER_ORDER.stars);
});

it('lights both bodies from their own bearing rather than from the sun', () => {
  const { gasGiant, moon } = createCelestialBodies();
  for (const mesh of [gasGiant, moon]) {
    expect(Object.keys(mesh.material.uniforms)).not.toContain('uSun');
    const light = mesh.material.uniforms.uLight.value as THREE.Vector3;
    expect(light.length()).toBeCloseTo(1, 6);
    // The face turned to the forum is the modelled one: the surface normal of
    // the point facing the origin must be on the lit side of the terminator.
    const facing = mesh.position.clone().negate().normalize();
    expect(light.dot(facing)).toBeGreaterThan(0.4);
    expect(mesh.material.fragmentShader).not.toContain('uSun');
  }
  // ...and the gas giant's limb glow no longer scales with a sun-side term.
  expect(gasGiant.material.fragmentShader).toContain('rim');
  expect(gasGiant.material.fragmentShader).not.toContain('sunSide');
});

it('hangs the gas giant above and clear of the ridge, and inside the default vista', () => {
  expect(elevation(GAS_GIANT_POS)).toBeGreaterThanOrEqual(26);
  expect(elevation(GAS_GIANT_POS)).toBeLessThanOrEqual(30);
  // Still clear of the landform: the ridge mesh exists only between Blender
  // polar 100 and 215 degrees, and the giant now sits short of 100 rather than
  // past 215. It moved because the default vantage faces north (job 20) and a
  // south-westerly planet is behind the viewer, not in the vista.
  const bearing = blenderBearing(GAS_GIANT_POS);
  expect(bearing).toBeLessThan(RIDGE_ARC[0]);
  expect(bearing).toBeGreaterThan(60);
  // Northerly in three.js axes: -z is the Blender +y north.
  expect(GAS_GIANT_POS.z).toBeLessThan(0);
  // Both bodies are in the default frame, on opposite sides of it.
  const giant = projectPoint([GAS_GIANT_POS.x, GAS_GIANT_POS.y, GAS_GIANT_POS.z])!;
  const satellite = projectPoint([MOON_POS.x, MOON_POS.y, MOON_POS.z])!;
  for (const ndc of [giant, satellite]) {
    expect(Math.abs(ndc.x)).toBeLessThan(1);
    expect(Math.abs(ndc.y)).toBeLessThan(1);
  }
  // High, and separated: the giant left of the moon with a clear gap between
  // the two discs rather than one crowding the other.
  expect(giant.y).toBeGreaterThan(0.5);
  expect(satellite.y).toBeGreaterThan(0.5);
  expect(satellite.x - giant.x).toBeGreaterThan(0.3);
});

it('keeps the moon smaller, paler and off the ridge too', () => {
  const { gasGiant, moon } = createCelestialBodies();
  expect(MOON_RADIUS).toBeLessThan(GAS_GIANT_RADIUS);
  expect((moon.geometry.parameters.radius)).toBe(MOON_RADIUS);
  expect((gasGiant.geometry.parameters.radius)).toBe(GAS_GIANT_RADIUS);
  const bearing = blenderBearing(MOON_POS);
  expect(bearing < RIDGE_ARC[0] || bearing > RIDGE_ARC[1]).toBe(true);
  expect(elevation(MOON_POS)).toBeGreaterThan(20);
  // Pale: both moon tones are near-neutral and bright (HSL here is read in
  // three's linear working space, not in sRGB), and the highlands are lighter
  // than the maria.
  const read = (color: THREE.Color) => { const hsl = { h: 0, s: 0, l: 0 }; color.getHSL(hsl); return hsl; };
  const high = read(moon.material.uniforms.uHigh.value as THREE.Color);
  const low = read(moon.material.uniforms.uLow.value as THREE.Color);
  for (const tone of [high, low]) {
    expect(tone.s).toBeLessThan(0.2);
    expect(tone.l).toBeGreaterThan(0.3);
  }
  expect(high.l).toBeGreaterThan(low.l);
  // Paler than the gas giant's warm belts, which is what tells the two apart.
  expect(high.s).toBeLessThan(read(gasGiant.material.uniforms.uWarm.value as THREE.Color).s);
  // Its own faint halo, not a borrowed one.
  expect(moon.material.uniforms.uGlow).toBeTruthy();
});

it('mounts both bodies in the night scene and in the day scene', () => {
  for (const appearance of ['night', 'day'] as const) {
    const host = document.createElement('div');
    const root = createRoot(host);
    act(() => root.render(<ForumSky appearance={appearance} />));
    for (const name of [CELESTIAL_NAMES.gasGiant, CELESTIAL_NAMES.moon]) {
      expect(host.querySelectorAll(`primitive[name="${name}"]`).length).toBe(1);
    }
    act(() => root.unmount());
  }
});

it('lights the night with moonlight rather than a warm fill, without touching the emissives', () => {
  const channels = (hex: string) => [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
  for (const key of ['sky', 'key', 'fill', 'ground'] as const) {
    const [r, , b] = channels(FORUM_LIGHT.night[key]);
    // Cool: every broad night light is bluer than it is red. The amber fill
    // that mixed with the violet hemisphere into lavender paving was the one
    // that failed this.
    expect(b, `night ${key} is not cool`).toBeGreaterThan(r);
  }
  // Day is untouched: its key is still the warm sunrise.
  const [dayR, , dayB] = channels(FORUM_LIGHT.day.key);
  expect(dayR).toBeGreaterThan(dayB);
});
