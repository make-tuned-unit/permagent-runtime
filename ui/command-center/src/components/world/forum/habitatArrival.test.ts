import { expect, it } from 'vitest';
import { HABITAT_ARRIVAL, HABITAT_ARRIVAL_REVERSE, HABITAT_OVERVIEW, HABITAT_START, HABITAT_STATIONS } from './habitatLayout';

type Vec3 = readonly [number, number, number];
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const sub = (a: Vec3, b: Vec3): [number, number, number] => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a: Vec3, b: Vec3): [number, number, number] => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const unit = (v: Vec3): [number, number, number] => { const length = Math.hypot(...v); return [v[0] / length, v[1] / length, v[2] / length]; };
function project(point: Vec3, camera: { position: Vec3; target: Vec3; fov: number }, aspect: number) {
  const forward = unit(sub(camera.target, camera.position));
  const right = unit(cross(forward, [0, 1, 0]));
  const up = cross(right, forward);
  const delta = sub(point, camera.position);
  const depth = dot(delta, forward);
  const scale = Math.tan(camera.fov * Math.PI / 360);
  return { x: dot(delta, right) / depth / (scale * aspect), y: dot(delta, up) / depth / scale, depth };
}

it('opens at eye height beside the host and table, aimed down the inhabited route', () => {
  expect(HABITAT_ARRIVAL.position[1]).toBeCloseTo(1.6, 1);
  const host = HABITAT_STATIONS[0].position;
  expect(Math.hypot(HABITAT_ARRIVAL.position[0] - host[0], HABITAT_ARRIVAL.position[2] - host[2])).toBeGreaterThan(2);
  expect(Math.hypot(HABITAT_ARRIVAL.position[0] - host[0], HABITAT_ARRIVAL.position[2] - host[2])).toBeLessThan(4);
  expect(HABITAT_ARRIVAL.target[2]).toBeLessThan(HABITAT_ARRIVAL.position[2]);
  expect(HABITAT_STATIONS.length).toBeGreaterThan(0);
});

it.each([1280 / 800, 1440 / 900])('keeps the compact footprint in the overview at aspect %s', aspect => {
  const footprint: Vec3[] = [];
  for (const x of [-16, 16]) for (const z of [16, -55]) for (const y of [0, 3.4]) footprint.push([x, y, z]);
  const projected = footprint.map(point => project(point, HABITAT_OVERVIEW, aspect));
  expect(projected.every(point => point.depth > 0 && Math.abs(point.x) < 1 && Math.abs(point.y) < 1)).toBe(true);
});

it('keeps the reverse arrival framed by planted rear depth', () => {
  const rearPlanting = [[-2, 2, 5.8], [3, 2, 8.5], [-1.5, 2, 11], [0, 2, 12.5]] as Vec3[];
  const points = rearPlanting.map(point => project(point, HABITAT_ARRIVAL_REVERSE, 1.6));
  expect(points.every(point => point.depth > 0 && Math.abs(point.x) < 1 && Math.abs(point.y) < 1)).toBe(true);
  expect(HABITAT_ARRIVAL_REVERSE.target[2]).toBeGreaterThan(HABITAT_START[2]);
});
