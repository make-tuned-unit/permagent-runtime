import { expect, it } from 'vitest';
import { PerspectiveCamera, Vector3 } from 'three';
import { HABITAT_OVERVIEW, HABITAT_PLACES, HABITAT_STATIONS, habitatOverviewCamera } from './habitatLayout';
function camera(aspect: number) {
  const framing = habitatOverviewCamera(aspect);
  const view = new PerspectiveCamera(framing.fov, aspect, .25, 6000);
  view.position.set(...framing.position); view.lookAt(new Vector3(...framing.target)); view.updateMatrixWorld();
  return view;
}
it.each([1.6, 1.3, .9, .6])('frames connected places, agents and both terraces at canvas aspect %s', aspect => {
  const view = camera(aspect);
  const points = [...HABITAT_PLACES.map(p => p.position), ...HABITAT_STATIONS.map(p => [p.position[0], p.position[1] + 1.6, p.position[2]])];
  for (const x of [-16, 18]) for (const z of [-55, 16]) for (const y of [0, 3.4]) points.push([x, y, z]);
  for (const point of points) {
    const projected = new Vector3(...point).project(view);
    expect(Math.abs(projected.x), `horizontal ${point}`).toBeLessThan(.92);
    expect(Math.abs(projected.y), `vertical ${point}`).toBeLessThan(.92);
    expect(projected.z).toBeLessThan(1);
  }
});
it('keeps the arrival roof ribs local to the commons instead of spanning the frame', () => {
  const view = camera(1.6);
  const projected = [-7.5, -.5].flatMap(x => [1, 6.6].map(z => new Vector3(x, 3.9, z).project(view)));
  const width = Math.max(...projected.map(p => p.x)) - Math.min(...projected.map(p => p.x));
  expect(width).toBeLessThan(.3); // <15% of the full frame width.
  expect(HABITAT_OVERVIEW.position[1]).toBeGreaterThan(11.8); // Above the great arch.
});
