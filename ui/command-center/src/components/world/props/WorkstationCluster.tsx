// W2 — WorkstationCluster: the Build wing's working bay (bible §3 A1).
// Two long stone bench rows × three stations: holo terminals, overhead gantry
// rail with amber task lights, engraved cable channels in the floor, stools.
// This is where agents physically sit to work — densest anchor concentration.
//
// Draw calls: 8 (bench tops · supports · seats · cylinder metalwork ·
// box metalwork · holo screens · task lights · floor channels).
// Anchors: 6 seat anchors (one per stool), registered via shared/anchors.

import { useMemo } from 'react';
import type { AreaId } from '../shared/anchors';
import type { AgentAnchor } from '../shared/anchors';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, unitCylinder8, unitCylinder16, unitPlane } from './geometries';
import {
  stoneDark,
  stoneMarble,
  metalGunmetal,
  holoCyan,
  lightAmber,
  lightCyan,
} from './materials';
import { placeAnchor, useRegisterAnchors } from './propUtils';

const ROW_Z = [0, -3.0];
const STATION_X = [-2.4, 0, 2.4];
const BENCH_Y = 0.92;
const SEAT_LABEL = ['A', 'B', 'C'];

interface WorkstationClusterProps {
  position?: [number, number, number];
  rotationY?: number;
  areaId?: AreaId;
  /** Anchor id + instance-ledger prefix, e.g. 'build.cluster1'. */
  idPrefix?: string;
}

export function WorkstationCluster({
  position = [0, 0, 0],
  rotationY = 0,
  areaId = 'build',
  idPrefix = 'build.cluster1',
}: WorkstationClusterProps) {
  const layout = useMemo(() => {
    const benchTops: InstanceTransform[] = [];
    const supports: InstanceTransform[] = [];
    const seats: InstanceTransform[] = [];
    const metalCyl: InstanceTransform[] = [];
    const metalBox: InstanceTransform[] = [];
    const screens: InstanceTransform[] = [];
    const taskLights: InstanceTransform[] = [];
    const channels: InstanceTransform[] = [];

    for (const z of ROW_Z) {
      // Bench top + marble supports.
      benchTops.push({ position: [0, BENCH_Y, z], scale: [7.6, 0.12, 0.9] });
      for (const sx of [-3.5, 0, 3.5]) {
        supports.push({ position: [sx, 0.43, z], scale: [0.22, 0.86, 0.7] });
      }

      // Overhead gantry: two posts + rail carrying the task lights.
      for (const px of [-3.7, 3.7]) {
        metalCyl.push({ position: [px, 1.4, z - 0.25], scale: [0.14, 2.8, 0.14] });
      }
      metalBox.push({ position: [0, 2.82, z - 0.25], scale: [7.7, 0.07, 0.14] });

      // Cable run engraved along the row.
      channels.push({ position: [0, 0.012, z - 0.6], scale: [7.6, 0.02, 0.1] });

      for (const x of STATION_X) {
        // Stool: round stone seat on gunmetal stem + base.
        seats.push({ position: [x, 1.0, z + 0.95], scale: [0.9, 0.07, 0.9] });
        metalCyl.push({ position: [x, 0.5, z + 0.95], scale: [0.12, 0.94, 0.12] });
        metalCyl.push({ position: [x, 0.03, z + 0.95], scale: [0.55, 0.06, 0.55] });

        // Holo terminal: gunmetal frame + cyan holo pane, tilted back.
        metalBox.push({
          position: [x, 1.55, z - 0.25],
          rotation: [-0.15, 0, 0],
          scale: [0.95, 0.7, 0.06],
        });
        screens.push({
          position: [x, 1.55, z - 0.21],
          rotation: [-0.15, 0, 0],
          scale: [0.84, 0.58, 1],
        });

        // Amber task light under the rail; channel spur to the terminal.
        taskLights.push({ position: [x, 2.76, z - 0.25], scale: [0.8, 0.05, 0.12] });
        channels.push({ position: [x, 0.012, z - 0.25], scale: [0.08, 0.02, 0.62] });
      }
    }

    return { benchTops, supports, seats, metalCyl, metalBox, screens, taskLights, channels };
  }, []);

  const anchors = useMemo<AgentAnchor[]>(
    () =>
      ROW_Z.flatMap((z, r) =>
        STATION_X.map((x, s) => {
          const placed = placeAnchor([x, 0, z + 0.95], Math.PI, position, rotationY);
          return {
            id: `${idPrefix}.bench${r + 1}.seat${SEAT_LABEL[s]}`,
            areaId,
            kind: 'seat' as const,
            ...placed,
          };
        })
      ),
    [areaId, idPrefix, position, rotationY]
  );
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      <InstancedProp name={`${idPrefix}.benchTop`} geometry={unitBox} material={stoneDark} transforms={layout.benchTops} castShadow />
      <InstancedProp name={`${idPrefix}.benchSupport`} geometry={unitBox} material={stoneMarble} transforms={layout.supports} />
      <InstancedProp name={`${idPrefix}.stoolSeat`} geometry={unitCylinder16} material={stoneDark} transforms={layout.seats} castShadow />
      <InstancedProp name={`${idPrefix}.metalCyl`} geometry={unitCylinder8} material={metalGunmetal} transforms={layout.metalCyl} />
      <InstancedProp name={`${idPrefix}.metalBox`} geometry={unitBox} material={metalGunmetal} transforms={layout.metalBox} />
      <InstancedProp name={`${idPrefix}.holoScreen`} geometry={unitPlane} material={holoCyan} transforms={layout.screens} />
      <InstancedProp name={`${idPrefix}.taskLight`} geometry={unitBox} material={lightAmber} transforms={layout.taskLights} />
      <InstancedProp name={`${idPrefix}.floorChannel`} geometry={unitBox} material={lightCyan} transforms={layout.channels} />
    </group>
  );
}
