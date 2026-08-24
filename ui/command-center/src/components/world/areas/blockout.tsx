// Blockout helpers — gray matte geometry for the zone blockout milestone.
// Stone tier (bible §1): matte, never emissive. One shared material + one box
// geometry; every zone shell is a single InstancedMesh draw call.

import { useMemo } from 'react';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';

/** Shared Stone-tier blockout material (module singleton — one program, no dupes). */
export const blockoutMat = new THREE.MeshLambertMaterial({
  color: ENV.darkStone,
});

/** Shared unit cube; instances scale it into floors/walls/ceilings. */
export const unitBox = new THREE.BoxGeometry(1, 1, 1);

export interface ZoneShellProps {
  /** Ledger name, e.g. 'build.shell'. */
  name: string;
  /** Room center along local +x (zone modules render in threshold-local space). */
  cx: number;
  /** Footprint: depth along local x, width along local z. */
  depth: number;
  width: number;
  /** Wall height (side-area ceilings are 6–9u per bible §1). */
  height: number;
  /** Causeway from the colonnade line to the room's open side. */
  causeway?: { fromX: number; width: number };
}

const WALL_T = 0.5;
const SLAB_T = 0.4;

/**
 * Gray room shell: floor slab, back wall, two side walls, ceiling slab,
 * optional causeway — all instances of one unit box (1 draw call).
 * Open side faces local -x (toward the hall).
 */
export function ZoneShell({ name, cx, depth, width, height, causeway }: ZoneShellProps) {
  const transforms = useMemo<InstanceTransform[]>(() => {
    const t: InstanceTransform[] = [
      // Floor slab
      { position: [cx, -SLAB_T / 2, 0], scale: [depth, SLAB_T, width] },
      // Back wall (+x)
      { position: [cx + depth / 2 - WALL_T / 2, height / 2, 0], scale: [WALL_T, height, width] },
      // Side walls (±z)
      { position: [cx, height / 2, width / 2 - WALL_T / 2], scale: [depth, height, WALL_T] },
      { position: [cx, height / 2, -width / 2 + WALL_T / 2], scale: [depth, height, WALL_T] },
      // Ceiling slab
      { position: [cx, height + SLAB_T / 2, 0], scale: [depth, SLAB_T, width] },
    ];
    if (causeway) {
      const len = cx - depth / 2 - causeway.fromX;
      if (len > 0) {
        t.push({
          position: [causeway.fromX + len / 2, -SLAB_T / 2, 0],
          scale: [len, SLAB_T, causeway.width],
        });
      }
    }
    return t;
  }, [name, cx, depth, width, height, causeway]);

  return (
    <InstancedProp
      name={name}
      geometry={unitBox}
      material={blockoutMat}
      transforms={transforms}
      receiveShadow
    />
  );
}
