import { describe, expect, it } from 'vitest';
import { Mesh, Vector3 } from 'three';
import { OBSERVATORY_BRIDGE, OBSERVATORY_PLINTH, OBSERVATORY_WING_HEIGHT } from './habitatObservatoryWing';
import { HABITAT_DECOR, HABITAT_START, HABITAT_STATIONS, decorSolidSize, habitatCollisionSource, habitatElevation, habitatWalkable, type Point3 } from './habitatLayout';
import { findHabitatPath } from './habitatMotion';
import { attemptWalk, registerForumCollision } from './walkCollision';

function disposeSource(source: ReturnType<typeof habitatCollisionSource>) {
  const materials = new Set<{ dispose: () => void }>();
  source.traverse(object => { if (object instanceof Mesh) { object.geometry.dispose(); (Array.isArray(object.material) ? object.material : [object.material]).forEach(material => materials.add(material)); } });
  materials.forEach(material => material.dispose());
}

function walkRoute(start: Point3, target: Point3) {
  const path = findHabitatPath(start, target);
  expect(path, `route to ${target}`).not.toBeNull();
  const feet = new Vector3(...start);
  for (const point of path!) {
    let steps = 0; const maxSteps = Math.ceil(Math.hypot(feet.x - point[0], feet.z - point[2]) / .1) + 10;
    while (Math.hypot(feet.x - point[0], feet.z - point[2]) > .025) {
      expect(++steps, `progress toward ${point}`).toBeLessThanOrEqual(maxSteps);
      const dx = point[0] - feet.x, dz = point[2] - feet.z, distance = Math.hypot(dx, dz), step = Math.min(.1, distance);
      expect(attemptWalk(feet, dx / distance * step, dz / distance * step), `obstruction from ${start} to ${target} at ${feet.toArray()} toward ${point}`).toBe(true);
    }
  }
  expect(Math.hypot(feet.x - target[0], feet.z - target[2])).toBeLessThan(.08);
}

function accessPoint(p: (typeof HABITAT_DECOR)[number]): Point3 | null {
  const size = decorSolidSize(p), clearance = Math.max(size[0], size[2]) / 2 + .9;
  const candidates: Point3[] = [];
  for (let ring = 0; ring < 6; ring++) for (let index = 0; index < 24; index++) { const angle = index * Math.PI / 12; const radius = clearance + ring * .6; const x = p.position[0] + Math.cos(angle) * radius, z = p.position[2] + Math.sin(angle) * radius; candidates.push([x, habitatElevation(x, z), z]); }
  return candidates.find(candidate => habitatWalkable(candidate[0], candidate[2]) && findHabitatPath(HABITAT_START, candidate) !== null && findHabitatPath(candidate, HABITAT_START) !== null) ?? null;
}

describe('complete compact habitat traversal', () => {
  it('walks every station, both bridge ends, the observatory court, and every bench/planter/tree access point in both directions', () => {
    const source = habitatCollisionSource(), release = registerForumCollision(source);
    try {
      const access = HABITAT_DECOR.filter(p => ['bench', 'planter', 'canopy'].includes(p.asset)).map(accessPoint);
      expect(access.every(Boolean), `every solid decor has a walkable approach; missing ${HABITAT_DECOR.filter(p => ['bench', 'planter', 'canopy'].includes(p.asset)).map((p, i) => access[i] ? null : `${p.asset}@${p.position.join(',')}`).filter(Boolean).join(' | ')}`).toBe(true);
      const destinations: Point3[] = [
        ...HABITAT_STATIONS.map(station => station.position),
        [4, OBSERVATORY_WING_HEIGHT, OBSERVATORY_BRIDGE.z1 - .7],
        [4, OBSERVATORY_WING_HEIGHT, OBSERVATORY_BRIDGE.z0 + .7],
        [OBSERVATORY_PLINTH.x + 3.4, OBSERVATORY_WING_HEIGHT, OBSERVATORY_PLINTH.z],
        ...access.filter((point): point is Point3 => point !== null),
      ];
      for (const destination of destinations) {
        walkRoute(HABITAT_START, destination);
        walkRoute(destination, HABITAT_START);
      }
    } finally { release(); disposeSource(source); }
  }, 120000);
});
