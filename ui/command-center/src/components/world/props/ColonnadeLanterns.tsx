// ColonnadeLanterns — bronze lantern fixtures on the colonnade columns.
//
// Decorative Light-tier craft (claims no agent state): each surviving column
// carries a small bracketed lantern facing the hall. Their warmth follows the
// REAL local clock (atmosphere/timeOfDay): banked embers at midday, the
// hall's warm ring after dusk. This is the detail layer that rewards looking —
// the lanterns are how you notice the world keeps your hours.
//
// BUDGET: 3 instanced draw calls (brackets · cages · cores), zero lights —
// the §1 rule that emissive materials carry "lit" for free. LAW: zero
// per-frame allocations (one damped scalar write on a shared material).

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { COLUMN_COUNT, ROTUNDA_RADIUS, DOME_HEIGHT } from '../constants';
import { isPunchedAngle } from '../areas/zones';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, unitCylinder8 } from './geometries';
import { metalBronze, stoneDark, lightLantern } from './materials';
import { getTimeOfDay } from '../atmosphere/timeOfDay';

const COL_R = ROTUNDA_RADIUS - 1;
const LANTERN_Y = Math.min(6.2, DOME_HEIGHT * 0.35);
/** Lantern hangs this far inward of the column face, toward the hall. */
const INSET = 1.05;

interface Layout {
  brackets: InstanceTransform[];
  cages: InstanceTransform[];
  cores: InstanceTransform[];
}

function buildLayout(): Layout {
  const brackets: InstanceTransform[] = [];
  const cages: InstanceTransform[] = [];
  const cores: InstanceTransform[] = [];
  for (let i = 0; i < COLUMN_COUNT; i++) {
    const a = (i / COLUMN_COUNT) * Math.PI * 2;
    if (isPunchedAngle(a)) continue;
    const cx = Math.cos(a) * COL_R;
    const cz = Math.sin(a) * COL_R;
    // Inward direction (toward hall center).
    const ix = -Math.cos(a);
    const iz = -Math.sin(a);
    // Yaw that maps local +x onto the inward radial: (cos ry, -sin ry) = (-cos a, -sin a).
    const yaw = Math.PI - a;
    // Bracket arm: the Y-rod tipped onto its side (Rz π/2 → lies along local x),
    // then yawed so it reaches inward from the column face.
    brackets.push({
      position: [cx + ix * (0.7 + INSET / 2), LANTERN_Y + 0.22, cz + iz * (0.7 + INSET / 2)],
      rotation: [0, yaw, Math.PI / 2],
      scale: [0.06, INSET, 0.06],
    });
    const lx = cx + ix * (0.7 + INSET);
    const lz = cz + iz * (0.7 + INSET);
    // Cage: a slim dark housing.
    cages.push({
      position: [lx, LANTERN_Y, lz],
      rotation: [0, yaw, 0],
      scale: [0.22, 0.34, 0.22],
    });
    // Core: the emissive heart (time-of-day owns its intensity).
    cores.push({
      position: [lx, LANTERN_Y, lz],
      scale: [0.1, 0.16, 0.1],
    });
  }
  return { brackets, cages, cores };
}

export function ColonnadeLanterns() {
  const layout = useMemo(buildLayout, []);
  const level = useRef(1.2);

  useFrame((_, dt) => {
    // Damp toward the real-clock lantern warmth (30s-cadence retargets).
    const target = getTimeOfDay().lanternGlow;
    level.current = THREE.MathUtils.damp(level.current, target, 1.5, dt);
    lightLantern.emissiveIntensity = level.current;
  });

  return (
    <group>
      <InstancedProp name="hall.lantern.bracket" geometry={unitCylinder8} material={metalBronze} transforms={layout.brackets} />
      <InstancedProp name="hall.lantern.cage" geometry={unitBox} material={stoneDark} transforms={layout.cages} />
      <InstancedProp name="hall.lantern.core" geometry={unitBox} material={lightLantern} transforms={layout.cores} />
    </group>
  );
}
