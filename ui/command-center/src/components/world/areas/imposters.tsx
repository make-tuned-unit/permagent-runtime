// Zone imposters — WORLD_VIEW_BIBLE.md §3/§8: an unloaded zone shows fog and
// the landmark's distant silhouette, ≤3 meshes. These live OUTSIDE the lazy
// chunks (always loaded, always cheap) and render in threshold-local space
// (+x away from hall center; ZoneMount applies the rotation).

import { blockoutMat } from './blockout';

/** Build: gantry crane mast + jib (2 meshes). */
export function BuildImposter() {
  return (
    <group position={[26, 0, 3.5]}>
      <mesh position-y={3} material={blockoutMat}>
        <boxGeometry args={[0.7, 6, 0.7]} />
      </mesh>
      <mesh position={[-3.2, 5.65, 0]} material={blockoutMat}>
        <boxGeometry args={[7.5, 0.5, 0.6]} />
      </mesh>
    </group>
  );
}

/** Brain: memory-core obelisk + one index ring (3 meshes). */
export function BrainImposter() {
  return (
    <group position={[25, 0, 0]}>
      <mesh position-y={3.4} material={blockoutMat}>
        <cylinderGeometry args={[0.45, 0.95, 5.6, 8]} />
      </mesh>
      <mesh position-y={6.55} material={blockoutMat}>
        <coneGeometry args={[0.45, 0.9, 8]} />
      </mesh>
      <mesh position-y={3.6} rotation-x={Math.PI / 2} material={blockoutMat}>
        <torusGeometry args={[1.7, 0.035, 4, 24]} />
      </mesh>
    </group>
  );
}

/** Lab: orrery outer ring + orb (2 meshes). */
export function LabImposter() {
  return (
    <group position={[24, 3.6, 0]}>
      <mesh material={blockoutMat}>
        <torusGeometry args={[2.4, 0.08, 6, 24]} />
      </mesh>
      <mesh material={blockoutMat}>
        <sphereGeometry args={[0.4, 8, 8]} />
      </mesh>
    </group>
  );
}

/** Automate: horologium outer ring + back-wall slab (2 meshes). */
export function AutomateImposter() {
  return (
    <group>
      <mesh position={[29.4, 4, 0]} rotation-y={Math.PI / 2} material={blockoutMat}>
        <torusGeometry args={[3.0, 0.1, 6, 24]} />
      </mesh>
      <mesh position={[29.7, 3.5, 0]} material={blockoutMat}>
        <boxGeometry args={[0.4, 7, 10]} />
      </mesh>
    </group>
  );
}

/** Antechamber: dormant ring + chamber mass behind the gate (2 meshes). */
export function AntechamberImposter() {
  return (
    <group>
      <mesh position={[29.2, 2.6, 0]} rotation-y={Math.PI / 2} material={blockoutMat}>
        <torusGeometry args={[2.0, 0.18, 6, 24]} />
      </mesh>
      <mesh position={[26.9, 2.8, 0]} material={blockoutMat}>
        <boxGeometry args={[10, 5.8, 10]} />
      </mesh>
    </group>
  );
}
