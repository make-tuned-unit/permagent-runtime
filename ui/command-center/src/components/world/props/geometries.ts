// W2 prop library — shared unit geometries.
// WORLD_VIEW_BIBLE.md §1 (big simple masses), §8 (instancing law).
//
// Module-lifetime singletons: created once at import, NEVER disposed. Every
// instanced prop family scales these unit shapes per instance instead of
// allocating bespoke BufferGeometries — geometry count stays flat no matter
// how many props mount/unmount (leak-consciousness per bible §0.8).

import * as THREE from 'three';

/** 1×1×1 box centered at origin. The workhorse: slabs, benches, books, steles. */
export const unitBox = new THREE.BoxGeometry(1, 1, 1);

/** r=0.5 h=1 octagonal cylinder. Legs, stems, posts, channels. */
export const unitCylinder8 = new THREE.CylinderGeometry(0.5, 0.5, 1, 8);

/** r=0.5 h=1 16-gon cylinder. Round seats, plinths, table pillars. */
export const unitCylinder16 = new THREE.CylinderGeometry(0.5, 0.5, 1, 16);

/** Tapered cylinder (0.35 top / 0.5 bottom). Flasks, brazier bowls. */
export const unitTaperedCylinder8 = new THREE.CylinderGeometry(0.35, 0.5, 1, 8);

/** 1×1 plane facing +z. Holo screens, trace-wall panels. */
export const unitPlane = new THREE.PlaneGeometry(1, 1);

/** r=0.5 octahedron. Memory shards (Brain Archive violet crystals). */
export const unitShard = new THREE.OctahedronGeometry(0.5, 0);

/**
 * Deterministic hash → [0,1). Replaces Math.random() in legacy props so
 * instanced layouts are stable across mounts (and across evidence captures).
 */
export function hash01(n: number): number {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453123;
  return s - Math.floor(s);
}
