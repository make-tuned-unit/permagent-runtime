// W2 — BrainShelves: the Brain Archive's deep stacks (bible §3 A2).
// Two shelf canyons flanking a center aisle: instanced stack rows of packed
// book slabs with violet seams, four specimen plinths holding pulsing violet
// memory shards, and a consultation table. The only violet-dominant family.
//
// Draw calls: 10 (uprights · planks · 2 slab tones · violet seams · plinths ·
// shards · table top · marble supports/benches · shard pulse shares shards).
// Anchors: 2 table seats + 4 plinth stand anchors.
// Motion: shard emissive breath on lightVioletShard only; static under
// reduceMotion. Zero per-frame allocations.

import { useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import { getReduceMotion } from '../../../styles/tokens';
import type { AreaId } from '../shared/anchors';
import type { AgentAnchor } from '../shared/anchors';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, unitCylinder16, unitShard, hash01 } from './geometries';
import {
  stoneDark,
  stoneMarble,
  lightViolet,
  lightVioletShard,
  bookRamp,
} from './materials';
import { placeAnchor, useRegisterAnchors } from './propUtils';

const ROW_Z = [-3, -1.2, 1.2, 3];
const ROW_LEN = 8;
const ROW_HEIGHT = 3.2;
const PLANK_Y = [0.06, 0.86, 1.66, 2.46];
const PLINTH_X = [-3.2, -2, 2, 3.2];

function ShardPulse() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  useFrame(({ clock }) => {
    if (reduceMotion) return;
    lightVioletShard.emissiveIntensity = 1.7 + 0.3 * Math.sin(clock.elapsedTime * 1.2);
  });
  return null;
}

interface BrainShelvesProps {
  position?: [number, number, number];
  rotationY?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

export function BrainShelves({
  position = [0, 0, 0],
  rotationY = 0,
  areaId = 'brain',
  idPrefix = 'brain',
}: BrainShelvesProps) {
  const layout = useMemo(() => {
    const uprights: InstanceTransform[] = [];
    const planks: InstanceTransform[] = [];
    const slabsA: InstanceTransform[] = [];
    const slabsB: InstanceTransform[] = [];
    const seams: InstanceTransform[] = [];
    const plinths: InstanceTransform[] = [];
    const shards: InstanceTransform[] = [];
    const marbleWork: InstanceTransform[] = [];

    for (let r = 0; r < ROW_Z.length; r++) {
      const z = ROW_Z[r];
      // Uprights every 1.6u; planks span the row; top cap closes the stack.
      for (let u = 0; u <= 5; u++) {
        uprights.push({ position: [-4 + u * 1.6, ROW_HEIGHT / 2, z], scale: [0.12, ROW_HEIGHT, 0.5] });
      }
      for (const y of PLANK_Y) {
        planks.push({ position: [0, y, z], scale: [ROW_LEN + 0.1, 0.06, 0.5] });
        // Violet seam engraved along both plank faces.
        for (const face of [-0.26, 0.26]) {
          seams.push({ position: [0, y + 0.045, z + face], scale: [ROW_LEN + 0.1, 0.025, 0.03] });
        }
      }
      planks.push({ position: [0, ROW_HEIGHT + 0.06, z], scale: [ROW_LEN + 0.1, 0.06, 0.5] });

      // Packed book slabs, one per bay per level, two alternating matte tones.
      for (let bay = 0; bay < 5; bay++) {
        const bx = -3.2 + bay * 1.6;
        for (let lvl = 0; lvl < PLANK_Y.length; lvl++) {
          const slab: InstanceTransform = {
            position: [bx, PLANK_Y[lvl] + 0.03 + 0.275, ROW_Z[r]],
            scale: [1.35, 0.55, 0.42],
          };
          if (Math.floor(hash01(r * 53 + bay * 11 + lvl) * 2) === 0) slabsA.push(slab);
          else slabsB.push(slab);
        }
      }
    }

    // Specimen plinths + memory shards along the center aisle.
    for (let i = 0; i < PLINTH_X.length; i++) {
      const x = PLINTH_X[i];
      plinths.push({ position: [x, 0.55, 0], scale: [0.7, 1.1, 0.7] });
      shards.push({
        position: [x, 1.48, 0],
        rotation: [0.22, hash01(i * 17) * Math.PI, 0.18],
        scale: [0.34, 0.55, 0.34],
      });
    }

    // Consultation table: dark stone top on marble slab supports, two bench seats.
    marbleWork.push({ position: [-0.9, 0.42, 0], scale: [0.18, 0.84, 1.0] });
    marbleWork.push({ position: [0.9, 0.42, 0], scale: [0.18, 0.84, 1.0] });
    marbleWork.push({ position: [0, 0.5, 1.0], scale: [1.8, 0.1, 0.45] }); // bench N
    marbleWork.push({ position: [0, 0.5, -1.0], scale: [1.8, 0.1, 0.45] }); // bench S
    marbleWork.push({ position: [-0.7, 0.25, 1.0], scale: [0.14, 0.5, 0.4] });
    marbleWork.push({ position: [0.7, 0.25, 1.0], scale: [0.14, 0.5, 0.4] });
    marbleWork.push({ position: [-0.7, 0.25, -1.0], scale: [0.14, 0.5, 0.4] });
    marbleWork.push({ position: [0.7, 0.25, -1.0], scale: [0.14, 0.5, 0.4] });

    return { uprights, planks, slabsA, slabsB, seams, plinths, shards, marbleWork };
  }, []);

  const anchors = useMemo<AgentAnchor[]>(() => {
    const list: AgentAnchor[] = [
      {
        id: `${idPrefix}.table.seatN`,
        areaId,
        kind: 'seat',
        ...placeAnchor([0, 0, 1.0], Math.PI, position, rotationY),
      },
      {
        id: `${idPrefix}.table.seatS`,
        areaId,
        kind: 'seat',
        ...placeAnchor([0, 0, -1.0], 0, position, rotationY),
      },
    ];
    PLINTH_X.forEach((x, i) => {
      list.push({
        id: `${idPrefix}.plinth${i + 1}.stand`,
        areaId,
        kind: 'stand',
        ...placeAnchor([x, 0, 0.95], Math.PI, position, rotationY),
      });
    });
    return list;
  }, [areaId, idPrefix, position, rotationY]);
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      <ShardPulse />
      <InstancedProp name={`${idPrefix}.upright`} geometry={unitBox} material={stoneDark} transforms={layout.uprights} castShadow />
      <InstancedProp name={`${idPrefix}.plank`} geometry={unitBox} material={stoneDark} transforms={layout.planks} />
      <InstancedProp name={`${idPrefix}.slabA`} geometry={unitBox} material={bookRamp[4]} transforms={layout.slabsA} />
      <InstancedProp name={`${idPrefix}.slabB`} geometry={unitBox} material={bookRamp[1]} transforms={layout.slabsB} />
      <InstancedProp name={`${idPrefix}.seam`} geometry={unitBox} material={lightViolet} transforms={layout.seams} />
      <InstancedProp name={`${idPrefix}.plinth`} geometry={unitCylinder16} material={stoneMarble} transforms={layout.plinths} castShadow />
      <InstancedProp name={`${idPrefix}.shard`} geometry={unitShard} material={lightVioletShard} transforms={layout.shards} />
      <InstancedProp name={`${idPrefix}.marbleWork`} geometry={unitBox} material={stoneMarble} transforms={layout.marbleWork} />
      <mesh position={[0, 0.88, 0]} scale={[2.2, 0.1, 1.2]} geometry={unitBox} material={stoneDark} castShadow />
    </group>
  );
}
