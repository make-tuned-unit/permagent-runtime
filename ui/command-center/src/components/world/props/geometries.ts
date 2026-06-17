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
 * r=0.5 dodecahedron. Rough rock: banked rubble, boulders, mineral masses,
 * sledge loads (Cave stratum families). Scaled non-uniformly per instance for
 * variety — one geometry, any number of rocks.
 */
export const unitRock = new THREE.DodecahedronGeometry(0.5, 0);

/**
 * r=0.5 tetrahedron. Mineral glitter in raw rock, survey-marker caps, sharp
 * crystal accents. Sub-detail sized; one geometry, scattered per instance.
 */
export const unitTetra = new THREE.TetrahedronGeometry(0.5, 0);

/**
 * r=0.5 h=1 6-sided cone. The Librarian's lamp shade, crude tool wedges /
 * pick heads, the hauling sledge's nose, survey-marker plumb tips. Low-poly,
 * primal — fits the living-cave stratum silhouette grammar.
 */
export const unitCone6 = new THREE.ConeGeometry(0.5, 1, 6);

/**
 * Deterministic hash → [0,1). Replaces Math.random() in legacy props so
 * instanced layouts are stable across mounts (and across evidence captures).
 */
export function hash01(n: number): number {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453123;
  return s - Math.floor(s);
}
