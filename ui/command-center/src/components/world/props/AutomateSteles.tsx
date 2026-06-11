// W2 — AutomateSteles: the Automate Hall's scheduler gallery (bible §3 A4).
// Four stone steles engraved with glowing timetable grids (cyan cells, a few
// amber "active" rows) on bronze bases, plus a long planning table. Rhythmic
// and quiet: the amber tick is the only animation, pulsing on a slow clock.
//
// Draw calls: 7 (steles · bronze bases · cyan cells · amber tick cells ·
// table top · marble supports · floor channel).
// Anchors: 4 lean anchors (one per stele) + 2 stand anchors at the table.
// Motion: lightAmberTick emissive on a 4s clock; static under reduceMotion.

import { useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import { getReduceMotion } from '../../../styles/tokens';
import type { AreaId } from '../shared/anchors';
import type { AgentAnchor } from '../shared/anchors';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, hash01 } from './geometries';
import { stoneDark, stoneMarble, metalBronze, lightCyan, lightAmberTick } from './materials';
import { placeAnchor, useRegisterAnchors } from './propUtils';

const STELE_X = [-4.5, -1.5, 1.5, 4.5];
const STELE_H = 3.2;
const GRID_ROWS = 6;
const GRID_COLS = 4;

function TickPulse() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  useFrame(({ clock }) => {
    if (reduceMotion) return;
    // Slow square-ish tick: bright for the first half of each 4s period.
    const phase = (clock.elapsedTime % 4) / 4;
    const target = phase < 0.5 ? 1.9 : 1.1;
    lightAmberTick.emissiveIntensity += (target - lightAmberTick.emissiveIntensity) * 0.08;
  });
  return null;
}

interface AutomateStelesProps {
  position?: [number, number, number];
  rotationY?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

export function AutomateSteles({
  position = [0, 0, 0],
  rotationY = 0,
  areaId = 'automate',
  idPrefix = 'automate',
}: AutomateStelesProps) {
  const layout = useMemo(() => {
    const steles: InstanceTransform[] = [];
    const bases: InstanceTransform[] = [];
    const cells: InstanceTransform[] = [];
    const ticks: InstanceTransform[] = [];
    const marbleWork: InstanceTransform[] = [];
    const channels: InstanceTransform[] = [];

    for (let s = 0; s < STELE_X.length; s++) {
      const x = STELE_X[s];
      steles.push({ position: [x, STELE_H / 2 + 0.18, -2.5], scale: [1.25, STELE_H, 0.3] });
      bases.push({ position: [x, 0.09, -2.5], scale: [1.45, 0.18, 0.5] });

      // Engraved timetable grid on the south (+z) face.
      for (let row = 0; row < GRID_ROWS; row++) {
        for (let col = 0; col < GRID_COLS; col++) {
          const cx = x - 0.42 + col * 0.28;
          const cy = 0.95 + row * 0.42;
          const cell: InstanceTransform = {
            position: [cx, cy, -2.34],
            scale: [0.2, 0.07, 0.02],
          };
          // A deterministic ~20% of cells are "active" amber ticks.
          if (hash01(s * 97 + row * 13 + col) < 0.2) ticks.push(cell);
          else cells.push(cell);
        }
      }
    }

    // Planning table: long dark stone top on marble slabs.
    marbleWork.push({ position: [-2.2, 0.42, 1.2], scale: [0.2, 0.84, 0.9] });
    marbleWork.push({ position: [2.2, 0.42, 1.2], scale: [0.2, 0.84, 0.9] });

    // Floor channel linking the steles to the table.
    channels.push({ position: [0, 0.012, -1.0], scale: [9.4, 0.02, 0.08] });

    return { steles, bases, cells, ticks, marbleWork, channels };
  }, []);

  const anchors = useMemo<AgentAnchor[]>(() => {
    const list: AgentAnchor[] = STELE_X.map((x, i) => ({
      id: `${idPrefix}.stele${i + 1}.lean`,
      areaId,
      kind: 'lean' as const,
      ...placeAnchor([x, 0, -1.7], Math.PI, position, rotationY),
    }));
    list.push({
      id: `${idPrefix}.table.standW`,
      areaId,
      kind: 'stand',
      ...placeAnchor([-1.2, 0, 2.1], Math.PI, position, rotationY),
    });
    list.push({
      id: `${idPrefix}.table.standE`,
      areaId,
      kind: 'stand',
      ...placeAnchor([1.2, 0, 2.1], Math.PI, position, rotationY),
    });
    return list;
  }, [areaId, idPrefix, position, rotationY]);
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      <TickPulse />
      <InstancedProp name={`${idPrefix}.stele`} geometry={unitBox} material={stoneDark} transforms={layout.steles} castShadow />
      <InstancedProp name={`${idPrefix}.base`} geometry={unitBox} material={metalBronze} transforms={layout.bases} />
      <InstancedProp name={`${idPrefix}.cell`} geometry={unitBox} material={lightCyan} transforms={layout.cells} />
      <InstancedProp name={`${idPrefix}.tick`} geometry={unitBox} material={lightAmberTick} transforms={layout.ticks} />
      <InstancedProp name={`${idPrefix}.marbleWork`} geometry={unitBox} material={stoneMarble} transforms={layout.marbleWork} />
      <InstancedProp name={`${idPrefix}.floorChannel`} geometry={unitBox} material={lightCyan} transforms={layout.channels} />
      <mesh position={[0, 0.88, 1.2]} scale={[5.2, 0.1, 1.1]} geometry={unitBox} material={stoneDark} castShadow />
    </group>
  );
}
