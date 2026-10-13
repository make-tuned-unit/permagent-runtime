import { Color, PerspectiveCamera, Ray, Sphere, Vector3 } from 'three';
import { expect, it } from 'vitest';
import { HABITAT_ATMOSPHERE, HABITAT_ATMOSPHERE_RADIUS, habitatAtmosphereFragment } from './HabitatSky';
import { habitatOverviewCamera } from './habitatLayout';

// Include both review windows and their narrow dock/desk canvas shapes. This
// checks actual camera rays against the backdrop, rather than only its radius.
it.each([[1280, 800], [1024, 640], [656, 772], [464, 612], [464, 280]])(
  'covers every overview ray inside the clipping range at %sx%s', (width, height) => {
    const framing = habitatOverviewCamera(width / height);
    const camera = new PerspectiveCamera(framing.fov, width / height, .25, 6000);
    camera.position.set(...framing.position); camera.lookAt(new Vector3(...framing.target)); camera.updateMatrixWorld();
    const atmosphere = new Sphere(new Vector3(), HABITAT_ATMOSPHERE_RADIUS);
    for (const x of [-1, -.5, 0, .5, 1]) for (const y of [-1, -.5, 0, .5, 1]) {
      const direction = new Vector3(x, y, .5).unproject(camera).sub(camera.position).normalize();
      const hit = new Ray(camera.position, direction).intersectSphere(atmosphere, new Vector3());
      expect(hit, `ray ${x},${y}`).not.toBeNull();
      const distance = camera.position.distanceTo(hit!);
      expect(distance).toBeGreaterThan(camera.near); expect(distance).toBeLessThan(camera.far);
      // The lower field and island-centre rays reach the opaque portion, so a
      // black panorama/fallback cannot show through underneath the island.
      if (y <= 0) expect(hit!.clone().normalize().y).toBeLessThan(-.22);
    }
  },
);

it('keeps both applied-lighting palettes above black without any time or texture dependency', () => {
  for (const palette of Object.values(HABITAT_ATMOSPHERE)) for (const value of Object.values(palette)) {
    const color = new Color(value);
    expect(Math.min(color.r, color.g, color.b)).toBeGreaterThan(.005);
    expect(Math.max(color.r, color.g, color.b)).toBeLessThan(.8);
  }
  expect(habitatAtmosphereFragment).not.toMatch(/\b(time|sampler2D|texture2D|sin|cos)\b/);
  expect(habitatAtmosphereFragment).toContain('1.-smoothstep(-.22,.12,y)');
});
