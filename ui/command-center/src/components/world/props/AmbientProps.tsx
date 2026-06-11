// W2 — AmbientProps: main-hall ambient furniture (bible §3 hall).
// A ring of marble benches between the columns (giving idle agents somewhere
// real to sit) and four bronze brazier bowls with ember-glow discs marking
// the cardinal thresholds. Quiet stone-and-bronze filler — no animation.
//
// Draw calls: 4 (bench slabs · bench supports · brazier bowls · ember discs).
// Anchors: 1 seat anchor per bench (6), facing the rotunda center.

import { useMemo } from 'react';
import type { AreaId } from '../shared/anchors';
import type { AgentAnchor } from '../shared/anchors';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, unitCylinder16, unitTaperedCylinder8 } from './geometries';
import { stoneMarble, metalBronze, lightAmber } from './materials';
import { useRegisterAnchors } from './propUtils';

const BENCH_COUNT = 6;
const BENCH_R = 12;
// Offset half a column bay so benches sit between the 8 columns, clear of
// the four cardinal thresholds.
const BENCH_ANGLES = Array.from(
  { length: BENCH_COUNT },
  (_, i) => (i / BENCH_COUNT) * Math.PI * 2 + Math.PI / 8
);
const BRAZIER_R = 8.5;
const BRAZIER_ANGLES = [Math.PI / 4, (3 * Math.PI) / 4, (5 * Math.PI) / 4, (7 * Math.PI) / 4];

interface AmbientPropsProps {
  areaId?: AreaId;
  idPrefix?: string;
}

export function AmbientProps({ areaId = 'hall', idPrefix = 'hall' }: AmbientPropsProps) {
  const layout = useMemo(() => {
    const slabs: InstanceTransform[] = [];
    const supports: InstanceTransform[] = [];
    const bowls: InstanceTransform[] = [];
    const embers: InstanceTransform[] = [];

    for (const a of BENCH_ANGLES) {
      const x = Math.cos(a) * BENCH_R;
      const z = Math.sin(a) * BENCH_R;
      // Tangent-aligned marble bench facing the rotunda center.
      const rot: [number, number, number] = [0, Math.PI / 2 - a, 0];
      slabs.push({ position: [x, 0.48, z], rotation: rot, scale: [1.7, 0.12, 0.55] });
      for (const off of [-0.6, 0.6]) {
        supports.push({
          position: [
            x + Math.cos(a + Math.PI / 2) * -off,
            0.21,
            z + Math.sin(a + Math.PI / 2) * -off,
          ],
          rotation: rot,
          scale: [0.14, 0.42, 0.5],
        });
      }
    }

    for (const a of BRAZIER_ANGLES) {
      const x = Math.cos(a) * BRAZIER_R;
      const z = Math.sin(a) * BRAZIER_R;
      bowls.push({ position: [x, 0.62, z], rotation: [Math.PI, 0, 0], scale: [0.9, 0.5, 0.9] });
      bowls.push({ position: [x, 0.19, z], scale: [0.34, 0.38, 0.34] });
      embers.push({ position: [x, 0.83, z], scale: [0.62, 0.05, 0.62] });
    }

    return { slabs, supports, bowls, embers };
  }, []);

  const anchors = useMemo<AgentAnchor[]>(
    () =>
      BENCH_ANGLES.map((a, i) => ({
        id: `${idPrefix}.bench${i + 1}.seat`,
        areaId,
        kind: 'seat' as const,
        position: [Math.cos(a) * BENCH_R, 0, Math.sin(a) * BENCH_R] as [number, number, number],
        // Face the rotunda center.
        facing: Math.atan2(-Math.cos(a), -Math.sin(a)),
      })),
    [areaId, idPrefix]
  );
  useRegisterAnchors(anchors);

  return (
    <group>
      <InstancedProp name={`${idPrefix}.benchSlab`} geometry={unitBox} material={stoneMarble} transforms={layout.slabs} castShadow />
      <InstancedProp name={`${idPrefix}.benchSupport`} geometry={unitBox} material={stoneMarble} transforms={layout.supports} />
      <InstancedProp name={`${idPrefix}.brazier`} geometry={unitTaperedCylinder8} material={metalBronze} transforms={layout.bowls} castShadow />
      <InstancedProp name={`${idPrefix}.ember`} geometry={unitCylinder16} material={lightAmber} transforms={layout.embers} />
    </group>
  );
}
