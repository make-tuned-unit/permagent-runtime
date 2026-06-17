// W2 — MezzanineBookWall: instanced replacement for the legacy BookshelfWall
// in WorldFurniture.tsx (~443 meshes: 1 wall + 6 planks + ~46 dividers +
// ~390 individually-meshed books) at 12 draw calls.
//
// Draw calls: 12 (wall segments · planks · dividers · amber trim · 8 book-
// color buckets). Books keep the varied-spine read via the fixed 8-tone
// matte ramp in materials.ts; layout is deterministic (hash01, not random).
//
// Geometry parameters mirror the legacy wall exactly (same radii, height,
// stair gap) so it drops in 1:1 during legacy migration. The legacy
// BookshelfWall is deleted from WorldFurniture.tsx in the migration pass
// after W1's skeleton lands — not in this PR (sequencing rule).

import { useMemo } from 'react';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, hash01 } from './geometries';
import { stoneDark, lightAmber, bookRamp } from './materials';

// Mirrors WorldFurniture.tsx mezzanine constants — single source after migration.
export const MEZZ_WALL_HEIGHT_Y = 10.01;
export const MEZZ_WALL_INNER_R = 12.5;
const WALL_R = MEZZ_WALL_INNER_R + 0.05;
const SHELF_MID_R = MEZZ_WALL_INNER_R + 0.2;
const WALL_HEIGHT = 4;
const SHELF_COUNT = 5;
const SEGMENTS = 48;
const BOOKS_PER_SHELF = 80;
const STAIR_GAP_CENTER = Math.PI * 0.375;
const STAIR_GAP_HALF = 0.12;

function isInStairGap(angle: number): boolean {
  let diff = angle - STAIR_GAP_CENTER;
  while (diff > Math.PI) diff -= Math.PI * 2;
  while (diff < -Math.PI) diff += Math.PI * 2;
  return Math.abs(diff) < STAIR_GAP_HALF;
}

/** Y rotation that lays a box's x-axis along the tangent at world angle `a`. */
function tangentRot(a: number): [number, number, number] {
  return [0, Math.PI / 2 - a, 0];
}

export function MezzanineBookWall() {
  const layout = useMemo(() => {
    const wall: InstanceTransform[] = [];
    const planks: InstanceTransform[] = [];
    const dividers: InstanceTransform[] = [];
    const trim: InstanceTransform[] = [];
    const books: InstanceTransform[][] = bookRamp.map(() => []);

    const shelfPitch = WALL_HEIGHT / SHELF_COUNT;

    for (let i = 0; i < SEGMENTS; i++) {
      const aMid = ((i + 0.5) / SEGMENTS) * Math.PI * 2;
      const aEdge = (i / SEGMENTS) * Math.PI * 2;

      // Back wall + shelf planks, segmented (skips the stair gap).
      if (!isInStairGap(aMid)) {
        wall.push({
          position: [Math.cos(aMid) * WALL_R, WALL_HEIGHT / 2, Math.sin(aMid) * WALL_R],
          rotation: tangentRot(aMid),
          scale: [1.66, WALL_HEIGHT, 0.06],
        });
        for (let s = 0; s <= SHELF_COUNT; s++) {
          planks.push({
            position: [Math.cos(aMid) * SHELF_MID_R, s * shelfPitch, Math.sin(aMid) * SHELF_MID_R],
            rotation: tangentRot(aMid),
            scale: [1.67, 0.05, 0.34],
          });
        }
        // Warm trim lip under each upper plank — the library's amber light.
        for (let s = 1; s <= SHELF_COUNT; s++) {
          trim.push({
            position: [
              Math.cos(aMid) * (SHELF_MID_R + 0.16),
              s * shelfPitch - 0.04,
              Math.sin(aMid) * (SHELF_MID_R + 0.16),
            ],
            rotation: tangentRot(aMid),
            scale: [1.6, 0.02, 0.02],
          });
        }
      }

      // Vertical dividers on segment edges.
      if (!isInStairGap(aEdge)) {
        dividers.push({
          position: [Math.cos(aEdge) * SHELF_MID_R, WALL_HEIGHT / 2, Math.sin(aEdge) * SHELF_MID_R],
          rotation: tangentRot(aEdge),
          scale: [0.05, WALL_HEIGHT, 0.32],
        });
      }
    }

    // Books — deterministic spines bucketed into the 8-tone ramp.
    const bookR = WALL_R + 0.12;
    for (let s = 0; s < SHELF_COUNT; s++) {
      const plankY = s * shelfPitch + 0.025;
      for (let b = 0; b < BOOKS_PER_SHELF; b++) {
        const angle = (b / BOOKS_PER_SHELF) * Math.PI * 2;
        if (isInStairGap(angle)) continue;
        const seed = s * 131 + b;
        const width = 0.06 + hash01(seed) * 0.05;
        const height = 0.42 + hash01(seed + 7) * 0.28;
        const bucket = Math.floor(hash01(seed + 13) * bookRamp.length);
        books[bucket].push({
          position: [Math.cos(angle) * bookR, plankY + height / 2, Math.sin(angle) * bookR],
          rotation: tangentRot(angle),
          scale: [width, height, 0.2],
        });
      }
    }

    return { wall, planks, dividers, trim, books };
  }, []);

  return (
    <group position-y={MEZZ_WALL_HEIGHT_Y}>
      <InstancedProp name="mezz.wall" geometry={unitBox} material={stoneDark} transforms={layout.wall} />
      <InstancedProp name="mezz.plank" geometry={unitBox} material={stoneDark} transforms={layout.planks} />
      <InstancedProp name="mezz.divider" geometry={unitBox} material={stoneDark} transforms={layout.dividers} />
      <InstancedProp name="mezz.trim" geometry={unitBox} material={lightAmber} transforms={layout.trim} />
      {layout.books.map((transforms, k) =>
        transforms.length > 0 ? (
          <InstancedProp
            key={k}
            name={`mezz.books${k}`}
            geometry={unitBox}
            material={bookRamp[k]}
            transforms={transforms}
          />
        ) : null
      )}
    </group>
  );
}
