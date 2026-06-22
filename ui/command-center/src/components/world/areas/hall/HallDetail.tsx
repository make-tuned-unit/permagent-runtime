// HallDetail — a second layer of architectural detail over the rotunda blockout.
// Cyber-classical craft: an entablature crowning the colonnade, fluted columns with
// neck/base moldings, a ribbed (coffered) dome, floor inlay (rim band + medallion),
// and a stepped platform rim. Repeated elements are instanced (one draw call each);
// no per-frame work — all geometry is built once in useMemo. Budget-safe (bible §8).

import { useMemo } from 'react';
import * as THREE from 'three';
import { COLORS, COLUMN_COUNT, ROTUNDA_RADIUS, DOME_HEIGHT, PLATFORM_RADIUS } from '../../constants';
import { isPunchedAngle } from '../zones';
import { ENV } from '../../shared/palette';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';

// ── Shared materials (module singletons — one program each) ──
const marble = new THREE.MeshStandardMaterial({ color: ENV.marble, roughness: 0.4, metalness: 0.05 });
const marbleShell = new THREE.MeshStandardMaterial({
  color: ENV.marble,
  roughness: 0.4,
  metalness: 0.05,
  side: THREE.DoubleSide,
});
const veinMarble = new THREE.MeshStandardMaterial({ color: ENV.marbleVein, roughness: 0.35, metalness: 0.18 });
const cyanLine = new THREE.MeshBasicMaterial({
  color: COLORS.neonCyan,
  transparent: true,
  opacity: 0.55,
  depthWrite: false,
  toneMapped: false,
});

// ── Shared geometries for instanced detail ──
const fluteGeo = new THREE.CylinderGeometry(0.035, 0.035, DOME_HEIGHT - 2.4, 5);
const neckGeo = new THREE.TorusGeometry(0.63, 0.05, 6, 24);
const baseGeo = new THREE.TorusGeometry(0.82, 0.1, 6, 24);

const COL_R = ROTUNDA_RADIUS - 1; // 14 — colonnade line

/** The colonnade columns (only the Mesh Stargate opening is punched out now). */
const COLUMNS: [number, number][] = Array.from({ length: COLUMN_COUNT }, (_, i) => (i / COLUMN_COUNT) * Math.PI * 2)
  .filter((a) => !isPunchedAngle(a))
  .map((a) => [Math.cos(a) * COL_R, Math.sin(a) * COL_R]);

// ── Entablature: the beam crowning the colonnade (architrave + cornice + frieze) ──
function Entablature() {
  const y = DOME_HEIGHT + 0.7;
  return (
    <group>
      <mesh position-y={y} material={marbleShell}>
        <cylinderGeometry args={[14.75, 14.75, 1.1, 48, 1, true]} />
      </mesh>
      <mesh position-y={y + 0.72} material={marbleShell}>
        <cylinderGeometry args={[15.15, 14.85, 0.42, 48, 1, true]} />
      </mesh>
      {/* Frieze accent line */}
      <mesh position-y={y} rotation-x={Math.PI / 2}>
        <torusGeometry args={[14.78, 0.05, 6, 72]} />
        <primitive object={cyanLine} attach="material" />
      </mesh>
    </group>
  );
}

// ── Column detail: vertical flutes + neck astragal + base torus, instanced ──
function ColumnDetail() {
  const flutes = useMemo<InstanceTransform[]>(() => {
    const t: InstanceTransform[] = [];
    const FLUTES = 12;
    for (const [x, z] of COLUMNS) {
      for (let k = 0; k < FLUTES; k++) {
        const a = (k / FLUTES) * Math.PI * 2;
        t.push({ position: [x + Math.cos(a) * 0.66, DOME_HEIGHT / 2, z + Math.sin(a) * 0.66] });
      }
    }
    return t;
  }, []);
  const necks = useMemo<InstanceTransform[]>(
    () => COLUMNS.map(([x, z]) => ({ position: [x, DOME_HEIGHT - 0.6, z], rotation: [Math.PI / 2, 0, 0] })),
    [],
  );
  const bases = useMemo<InstanceTransform[]>(
    () => COLUMNS.map(([x, z]) => ({ position: [x, 0.75, z], rotation: [Math.PI / 2, 0, 0] })),
    [],
  );
  return (
    <group>
      <InstancedProp name="hall.column.flutes" geometry={fluteGeo} material={marble} transforms={flutes} castShadow />
      <InstancedProp name="hall.column.necks" geometry={neckGeo} material={veinMarble} transforms={necks} />
      <InstancedProp name="hall.column.bases" geometry={baseGeo} material={marble} transforms={bases} />
    </group>
  );
}

// ── Dome ribs: latitude bands on the inner shell (coffered feel) ──
function DomeRibs() {
  const R = ROTUNDA_RADIUS + 1; // matches the dome shell radius
  const rings = useMemo(
    () => [3.5, 7, 10.5, 13].map((h) => ({ y: DOME_HEIGHT + h, r: Math.sqrt(Math.max(0.5, R * R - h * h)) - 0.1 })),
    [R],
  );
  return (
    <group>
      {rings.map((ring, i) => (
        <mesh key={i} position-y={ring.y} rotation-x={Math.PI / 2} material={veinMarble}>
          <torusGeometry args={[ring.r, 0.12, 6, 64]} />
        </mesh>
      ))}
    </group>
  );
}

// ── Floor inlay: a marble rim band, a cyan rim line, and a central medallion ──
function FloorInlay() {
  return (
    <group position-y={0.07}>
      <mesh rotation-x={-Math.PI / 2} material={veinMarble}>
        <ringGeometry args={[14.2, 14.85, 72]} />
      </mesh>
      <mesh rotation-x={-Math.PI / 2} position-y={0.01}>
        <ringGeometry args={[13.95, 14.08, 72]} />
        <primitive object={cyanLine} attach="material" />
      </mesh>
      {/* Central medallion */}
      <mesh rotation-x={-Math.PI / 2} material={veinMarble}>
        <circleGeometry args={[1.7, 40]} />
      </mesh>
      <mesh rotation-x={-Math.PI / 2} position-y={0.01}>
        <ringGeometry args={[1.55, 1.72, 48]} />
        <primitive object={cyanLine} attach="material" />
      </mesh>
    </group>
  );
}

// ── Platform rim: a stepped marble molding between the floor edge and the obsidian ──
function PlatformRim() {
  return (
    <group>
      <mesh position-y={-0.08} material={marbleShell}>
        <cylinderGeometry args={[15.35, 15.55, 0.5, 64, 1, true]} />
      </mesh>
      <mesh position-y={-0.32} material={marbleShell}>
        <cylinderGeometry args={[PLATFORM_RADIUS - 3, PLATFORM_RADIUS - 2.6, 0.45, 64, 1, true]} />
      </mesh>
    </group>
  );
}

export function HallDetail() {
  return (
    <group>
      <Entablature />
      <ColumnDetail />
      <DomeRibs />
      <FloorInlay />
      <PlatformRim />
    </group>
  );
}
