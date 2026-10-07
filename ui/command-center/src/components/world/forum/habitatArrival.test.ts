import { expect, it } from 'vitest';
import { HABITAT_ARRIVAL, HABITAT_ARRIVAL_REVERSE, HABITAT_OVERVIEW, HABITAT_SOLIDS, HABITAT_START, HABITAT_STATIONS, ORCHESTRATOR_TABLE_SEAT, stationFor, type HabitatSolid } from './habitatLayout';

type Vec3 = readonly [number, number, number];
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const sub = (a: Vec3, b: Vec3): [number, number, number] => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a: Vec3, b: Vec3): [number, number, number] => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const unit = (v: Vec3): [number, number, number] => { const length = Math.hypot(...v); return [v[0] / length, v[1] / length, v[2] / length]; };
const horizontalDistance = (a: Vec3, b: Vec3) => Math.hypot(a[0] - b[0], a[2] - b[2]);
const horizontalDistanceToSolid = (point: Vec3, solid: HabitatSolid) => {
  const dx = Math.max(Math.abs(point[0] - solid.at[0]) - solid.size[0] / 2, 0);
  const dz = Math.max(Math.abs(point[2] - solid.at[2]) - solid.size[2] / 2, 0);
  return Math.hypot(dx, dz);
};
function project(point: Vec3, camera: { position: Vec3; target: Vec3; fov: number }, aspect: number) {
  const forward = unit(sub(camera.target, camera.position));
  const right = unit(cross(forward, [0, 1, 0]));
  const up = cross(right, forward);
  const delta = sub(point, camera.position);
  const depth = dot(delta, forward);
  const scale = Math.tan(camera.fov * Math.PI / 360);
  return { x: dot(delta, right) / depth / (scale * aspect), y: dot(delta, up) / depth / scale, depth };
}

it('opens at eye height clear of every runtime body and solid, aimed down the inhabited route', () => {
  expect(HABITAT_ARRIVAL.position[1]).toBeGreaterThanOrEqual(1.55);
  expect(HABITAT_ARRIVAL.position[1]).toBeLessThanOrEqual(1.65);
  const host = stationFor('henry').position;
  expect(horizontalDistance(HABITAT_ARRIVAL.position, host)).toBeGreaterThanOrEqual(2);
  expect(horizontalDistance(HABITAT_ARRIVAL.position, host)).toBeLessThanOrEqual(4);

  // These are the same station/home points used by HeroGardenKit and the
  // habitat social home table; the extra seat is the ForumView "Sit with"
  // destination used by the orchestrator at the arrival table.
  const runtimeBodies = [
    ...HABITAT_STATIONS.map(station => ({id: `${station.id} station/home`, position: station.position})),
    {id: 'orchestrator table / Sit with seat', position: ORCHESTRATOR_TABLE_SEAT},
  ];
  for (const body of runtimeBodies) {
    expect(horizontalDistance(HABITAT_ARRIVAL.position, body.position), `${body.id} body`).toBeGreaterThanOrEqual(2);
  }
  for (const solid of HABITAT_SOLIDS) {
    expect(horizontalDistanceToSolid(HABITAT_ARRIVAL.position, solid), `${solid.name} solid`).toBeGreaterThanOrEqual(.6);
  }

  const framePoints = [
    ['host', [host[0], 1.55, host[2]]],
    ['arrival table', [-3, .989, 3]],
    ['water route', [2, .45, -4.5]],
  ] as const;
  for (const [label, point] of framePoints) {
    const projected = project(point, HABITAT_ARRIVAL, 1280 / 800);
    expect(projected.depth, `${label} depth`).toBeGreaterThan(0);
    expect(Math.abs(projected.x), `${label} horizontal frame`).toBeLessThan(1);
    expect(Math.abs(projected.y), `${label} vertical frame`).toBeLessThan(1);
  }
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
