// W2 — LabProps: the Lab's instrument benches + trace-wall (bible §3 A3).
// Two benches of glass instruments (holo-cyan flasks on gunmetal armatures)
// and a tall trace-wall panel streaming faint glyph strips. Cool and precise:
// cyan instrument glow against dark stone. (The orrery landmark and relocated
// telescope are W1 scope.)
//
// Draw calls: 8 (bench tops · marble supports · metal armatures · flasks ·
// trace frame · trace panel · glyph strips · floor channel).
// Anchors: 4 stand anchors (two per bench).
// Motion: none — glyphs are static engraved light (W4 owns ambience motion).

import { useMemo } from 'react';
import type { AreaId } from '../shared/anchors';
import type { AgentAnchor } from '../shared/anchors';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import {
  unitBox,
  unitCylinder8,
  unitTaperedCylinder8,
  unitPlane,
  hash01,
} from './geometries';
import {
  stoneDark,
  stoneMarble,
  metalGunmetal,
  holoCyan,
  lightCyan,
} from './materials';
import { placeAnchor, useRegisterAnchors } from './propUtils';

const BENCH_Z = [0.9, -1.7];
const BENCH_LEN = 5.2;
const STAND_X = [-1.3, 1.3];

interface LabPropsProps {
  position?: [number, number, number];
  rotationY?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

export function LabProps({
  position = [0, 0, 0],
  rotationY = 0,
  areaId = 'lab',
  idPrefix = 'lab',
}: LabPropsProps) {
  const layout = useMemo(() => {
    const benchTops: InstanceTransform[] = [];
    const supports: InstanceTransform[] = [];
    const armatures: InstanceTransform[] = [];
    const flasks: InstanceTransform[] = [];
    const glyphs: InstanceTransform[] = [];
    const channels: InstanceTransform[] = [];

    for (let b = 0; b < BENCH_Z.length; b++) {
      const z = BENCH_Z[b];
      benchTops.push({ position: [0, 0.92, z], scale: [BENCH_LEN, 0.1, 0.8] });
      for (const sx of [-2.3, 0, 2.3]) {
        supports.push({ position: [sx, 0.435, z], scale: [0.2, 0.87, 0.62] });
      }
      channels.push({ position: [0, 0.012, z + 0.55], scale: [BENCH_LEN, 0.02, 0.08] });

      // Instruments: tapered glass flasks on short gunmetal armatures.
      for (let i = 0; i < 6; i++) {
        const x = -2.1 + i * 0.84;
        const seed = b * 29 + i;
        const h = 0.26 + hash01(seed) * 0.22;
        armatures.push({ position: [x, 1.0, z - 0.12], scale: [0.16, 0.06, 0.16] });
        flasks.push({
          position: [x, 1.03 + h / 2, z - 0.12],
          scale: [0.22 + hash01(seed + 3) * 0.1, h, 0.22 + hash01(seed + 3) * 0.1],
        });
        // Tall retort tube on every third station.
        if (i % 3 === 1) {
          armatures.push({ position: [x + 0.3, 1.22, z + 0.15], scale: [0.05, 0.45, 0.05] });
        }
      }
    }

    // Trace-wall glyph strips: deterministic faint lines down the panel.
    for (let row = 0; row < 14; row++) {
      const y = 3.6 - row * 0.22;
      let x = -1.45;
      for (let g = 0; g < 3; g++) {
        const seed = row * 31 + g * 7;
        const len = 0.25 + hash01(seed) * 0.7;
        if (x + len > 1.45) break;
        glyphs.push({ position: [x + len / 2, y, -3.32], scale: [len, 0.045, 0.02] });
        x += len + 0.18 + hash01(seed + 5) * 0.25;
      }
    }

    return { benchTops, supports, armatures, flasks, glyphs, channels };
  }, []);

  const anchors = useMemo<AgentAnchor[]>(
    () =>
      BENCH_Z.flatMap((z, b) =>
        STAND_X.map((x, s) => ({
          id: `${idPrefix}.bench${b + 1}.stand${s === 0 ? 'L' : 'R'}`,
          areaId,
          kind: 'stand' as const,
          ...placeAnchor([x, 0, z + 0.85], Math.PI, position, rotationY),
        }))
      ),
    [areaId, idPrefix, position, rotationY]
  );
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      <InstancedProp name={`${idPrefix}.benchTop`} geometry={unitBox} material={stoneDark} transforms={layout.benchTops} castShadow />
      <InstancedProp name={`${idPrefix}.benchSupport`} geometry={unitBox} material={stoneMarble} transforms={layout.supports} />
      <InstancedProp name={`${idPrefix}.armature`} geometry={unitCylinder8} material={metalGunmetal} transforms={layout.armatures} />
      <InstancedProp name={`${idPrefix}.flask`} geometry={unitTaperedCylinder8} material={holoCyan} transforms={layout.flasks} />
      <InstancedProp name={`${idPrefix}.glyph`} geometry={unitBox} material={lightCyan} transforms={layout.glyphs} />
      <InstancedProp name={`${idPrefix}.floorChannel`} geometry={unitBox} material={lightCyan} transforms={layout.channels} />
      {/* Trace-wall: gunmetal frame + faint holo pane, glyphs engraved above. */}
      <mesh position={[0, 2.1, -3.4]} scale={[3.3, 3.8, 0.12]} geometry={unitBox} material={metalGunmetal} castShadow />
      <mesh position={[0, 2.1, -3.33]} scale={[3.0, 3.5, 1]} geometry={unitPlane} material={holoCyan} />
    </group>
  );
}
