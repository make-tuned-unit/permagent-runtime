// The Mouth — sightline aperture. GEOMETRY ONLY (W1).
// THE_CAVE_vision_bible.md §2: "a distant blade of pale daylight, far above the
// highest chamber … the light source for the entire world" = the Mesh portal.
// W4 owns the daylight shaft that pours through here (bible §9 W4); W1 ships only
// the rock shelf with the blade-slit cut through it, on the vertical sightline up
// out of the crown's oculus, so the geometry frames W4's light correctly.
//
// Gray blockout. Real scale. Placed far above DOME_HEIGHT (18) so it reads as
// "impossibly distant" (bible §7 hero shot). No emissive, no light — that is W4's.

import { useMemo } from 'react';
import * as THREE from 'three';
import { blockoutMat } from '../blockout';
import { MOUTH } from './strata';

// Shared unit cube for the lintel blocks (one geometry, scaled per block).
const unitCube = new THREE.BoxGeometry(1, 1, 1);

/**
 * A thick rock shelf with a blade-shaped slit cut through it. Built from four
 * solid lintel blocks framing the aperture (a CSG-free way to make a hole — the
 * slit is the gap between the blocks). The blade is long on x, narrow on z.
 */
export function MouthAperture() {
  const [mx, my, mz] = MOUTH.center;
  const blocks = useMemo(() => {
    const halfW = MOUTH.width / 2;
    const halfD = MOUTH.depth / 2;
    const th = MOUTH.rockThickness;
    // Shelf extends well beyond the slit so it reads as a solid ceiling of rock
    // pierced by one blade of light.
    const span = 26; // total rock shelf footprint (metres)
    const sideZ = span / 2 - (span / 2 - halfD) / 2; // centre of the ±z slabs
    const slabZ = (span / 2 - halfD); // depth of each ±z slab
    const endX = span / 2 - (span / 2 - halfW) / 2;
    const slabX = (span / 2 - halfW);
    return [
      // +z slab (beyond the slit's far edge)
      { pos: [0, 0, sideZ] as [number, number, number], scale: [span, th, slabZ] as [number, number, number] },
      // -z slab
      { pos: [0, 0, -sideZ] as [number, number, number], scale: [span, th, slabZ] as [number, number, number] },
      // +x end cap (closes the blade ends so the slit is finite length)
      { pos: [endX, 0, 0] as [number, number, number], scale: [slabX, th, MOUTH.depth] as [number, number, number] },
      // -x end cap
      { pos: [-endX, 0, 0] as [number, number, number], scale: [slabX, th, MOUTH.depth] as [number, number, number] },
    ];
  }, []);

  return (
    <group position={[mx, my, mz]}>
      {blocks.map((b, i) => (
        <mesh key={i} position={b.pos} scale={b.scale} geometry={unitCube} material={blockoutMat} />
      ))}
      {/* A thin frame lip around the slit — the carved edge of the Mouth. Reads as
          the worked threshold (the agents carved up to the daylight, bible §2). */}
      <mesh position={[0, -MOUTH.rockThickness / 2 - 0.15, 0]} material={blockoutMat}>
        <boxGeometry args={[MOUTH.width + 0.8, 0.3, MOUTH.depth + 0.8]} />
      </mesh>
    </group>
  );
}
