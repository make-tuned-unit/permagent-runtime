// CaveFloors — the WALKABLE surface of the cave (rebuild §5a/§5b).
// One continuous procedural ribbon that descends while spiralling around the throat
// well: the entry chute at the mouth → the flat chamber landings (the rooms) → the
// ramps between them. The geometry is sampled from the SAME analytic path used by
// floorYAt (strata.ts), so what you see is exactly what you stand on — no drift
// between the visual floor and the collision surface.
//
// Built once per strata change in useMemo (zero per-frame work). With one chamber this
// is the entry chute + verge landing; with N chambers the identical code produces the
// full verge→bedrock spiral.

import { useMemo } from 'react';
import * as THREE from 'three';
import { useStrata } from './strataState';
import {
  THROAT,
  WELL_RADIUS,
  MOUTH_BEARING,
  DESCENT_DIR,
  DESCENT_SWEEP,
  cavePathHeight,
  rampOuterAtY,
  type StratumDef,
} from './strata';

// Worked-stone floor — lighter/warmer than the raw cavern walls so the ground reads
// as something finished underfoot. The maturity gradient (verge warm → bedrock raw)
// is the craft pass (§7); this is the blockout surface. Module singleton.
const floorMat = new THREE.MeshStandardMaterial({
  color: '#4a4540',
  roughness: 0.92,
  metalness: 0.04,
  side: THREE.DoubleSide,
});

function buildSurface(strata: StratumDef[]): THREE.BufferGeometry {
  const n = Math.max(1, strata.length);
  const SWEEP_STEPS = n * 18; // along the descent
  const RADIAL_STEPS = 6; // across the annulus width
  const rows = RADIAL_STEPS + 1;
  const positions = new Float32Array((SWEEP_STEPS + 1) * rows * 3);
  const [cx, , cz] = THROAT.center;

  for (let i = 0; i <= SWEEP_STEPS; i++) {
    const u = i / SWEEP_STEPS;
    const y = cavePathHeight(u, strata);
    const outer = rampOuterAtY(y);
    const bearing = MOUTH_BEARING + DESCENT_DIR * u * DESCENT_SWEEP;
    const cosB = Math.cos(bearing);
    const sinB = Math.sin(bearing);
    for (let j = 0; j <= RADIAL_STEPS; j++) {
      const r = WELL_RADIUS + (outer - WELL_RADIUS) * (j / RADIAL_STEPS);
      const idx = (i * rows + j) * 3;
      positions[idx] = cx + cosB * r;
      positions[idx + 1] = y;
      positions[idx + 2] = cz + sinB * r;
    }
  }

  const indices: number[] = [];
  for (let i = 0; i < SWEEP_STEPS; i++) {
    for (let j = 0; j < RADIAL_STEPS; j++) {
      const a = i * rows + j;
      const b = (i + 1) * rows + j;
      const c = (i + 1) * rows + (j + 1);
      const d = i * rows + (j + 1);
      indices.push(a, b, d, b, c, d);
    }
  }

  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setIndex(indices);
  geo.computeVertexNormals();
  return geo;
}

export function CaveFloors() {
  const strata = useStrata();
  const geo = useMemo(() => buildSurface(strata), [strata]);
  return <mesh geometry={geo} material={floorMat} receiveShadow />;
}
