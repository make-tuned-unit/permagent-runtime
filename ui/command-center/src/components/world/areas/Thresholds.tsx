// Thresholds — WORLD_VIEW_BIBLE.md §3. Five portal frames punched through the
// colonnade: two columns + lintel + a floor seam in the zone's accent color.
// The colonnade column that sat on each threshold axis is removed in
// areas/hall (punched); two full-height flanking columns frame the doorway
// (instanced — 4 draw calls for all 8). The Forum Antechamber threshold is
// framed by the relocated Stargate instead of stone columns.

import { useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { getReduceMotion } from '../../../styles/tokens';
import { DOME_HEIGHT } from '../constants';
import { ZONES, COLONNADE_R } from './zones';
import { TextPlate } from './TextPlate';
import { StargatePortal } from './antechamber/Stargate';

const FLANK_OFFSET = 2.45; // ~10 deg of arc at r=14 — clear doorway ≈ 3.6u (≥3u law)
const LINTEL_Y = 5.0;

// Shared geometries/materials (module singletons — built once).
const bodyGeo = new THREE.CylinderGeometry(0.6, 0.7, DOME_HEIGHT, 16);
const veinGeo = new THREE.CylinderGeometry(0.15, 0.15, DOME_HEIGHT - 1, 8);
const capitalGeo = new THREE.CylinderGeometry(0.9, 0.6, 0.6, 16);
const baseGeo = new THREE.CylinderGeometry(0.7, 0.9, 0.6, 16);
const lintelGeo = new THREE.BoxGeometry(6.4, 0.6, 1.0);
const seamGeo = new THREE.BoxGeometry(7.5, 0.04, 0.5);

const marbleMat = new THREE.MeshStandardMaterial({ color: ENV.marble, roughness: 0.4, metalness: 0.05 });
const capitalMat = new THREE.MeshStandardMaterial({ color: ENV.marble, roughness: 0.3 });

export function Thresholds() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);

  const veinMat = useMemo(
    () => new THREE.MeshBasicMaterial({ color: ENV.neonCyan, transparent: true, opacity: 0.7 }),
    []
  );

  // Same slow pulse the colonnade veins use — one shared material, no allocations.
  useFrame(() => {
    if (reduceMotion) return;
    veinMat.opacity = 0.5 + 0.3 * Math.sin(performance.now() * 0.0008);
  });

  const { bodies, veins, capitals, bases, lintels } = useMemo(() => {
    const bodies: InstanceTransform[] = [];
    const veins: InstanceTransform[] = [];
    const capitals: InstanceTransform[] = [];
    const bases: InstanceTransform[] = [];
    const lintels: InstanceTransform[] = [];
    for (const z of ZONES) {
      if (z.id === 'antechamber') continue; // the Stargate frames this one
      const dx = Math.cos(z.angle);
      const dz = Math.sin(z.angle);
      const px = -dz;
      const pz = dx;
      for (const side of [-1, 1]) {
        const x = dx * COLONNADE_R + px * FLANK_OFFSET * side;
        const zz = dz * COLONNADE_R + pz * FLANK_OFFSET * side;
        bodies.push({ position: [x, DOME_HEIGHT / 2, zz] });
        veins.push({ position: [x, DOME_HEIGHT / 2, zz] });
        capitals.push({ position: [x, DOME_HEIGHT + 0.3, zz] });
        bases.push({ position: [x, 0.3, zz] });
      }
      lintels.push({
        position: [dx * COLONNADE_R, LINTEL_Y, dz * COLONNADE_R],
        rotation: [0, z.angle + Math.PI / 2, 0],
      });
    }
    return { bodies, veins, capitals, bases, lintels };
  }, []);

  const seams = useMemo(
    () =>
      ZONES.map((z) => ({
        zone: z,
        position: [Math.cos(z.angle) * 15.25, 0.14, Math.sin(z.angle) * 15.25] as [number, number, number],
        rotation: [0, -z.angle, 0] as [number, number, number],
        labelPos: [
          Math.cos(z.angle) * COLONNADE_R,
          z.id === 'antechamber' ? 7.6 : LINTEL_Y + 1.0,
          Math.sin(z.angle) * COLONNADE_R,
        ] as [number, number, number],
        labelRot: [0, Math.atan2(-Math.cos(z.angle), -Math.sin(z.angle)), 0] as [number, number, number],
      })),
    []
  );

  const antechamber = ZONES.find((z) => z.id === 'antechamber')!;

  return (
    <group>
      {/* Portal frame columns — instanced: 8 columns in 4 draw calls */}
      <InstancedProp name="thresholds.columnBody" geometry={bodyGeo} material={marbleMat} transforms={bodies} castShadow />
      <InstancedProp name="thresholds.columnVein" geometry={veinGeo} material={veinMat} transforms={veins} />
      <InstancedProp name="thresholds.columnCapital" geometry={capitalGeo} material={capitalMat} transforms={capitals} />
      <InstancedProp name="thresholds.columnBase" geometry={baseGeo} material={capitalMat} transforms={bases} />
      <InstancedProp name="thresholds.lintel" geometry={lintelGeo} material={marbleMat} transforms={lintels} />

      {/* Floor seams + zone labels — accent color leads the eye in (naming law §3) */}
      {seams.map(({ zone, position, rotation, labelPos, labelRot }) => (
        <group key={zone.id}>
          <mesh position={position} rotation={rotation} geometry={seamGeo}>
            <meshBasicMaterial color={zone.accent} transparent opacity={0.55} depthWrite={false} />
          </mesh>
          <TextPlate
            text={zone.label.toUpperCase()}
            height={0.55}
            color={zone.accent}
            opacity={0.85}
            position={labelPos}
            rotation={labelRot}
          />
        </group>
      ))}

      {/* The relocated Stargate frames the NW Antechamber threshold (§3 A5) */}
      <group
        position={[Math.cos(antechamber.angle) * 14.6, 0, Math.sin(antechamber.angle) * 14.6]}
        rotation-y={Math.PI / 2 - antechamber.angle}
      >
        <StargatePortal />
      </group>
    </group>
  );
}
