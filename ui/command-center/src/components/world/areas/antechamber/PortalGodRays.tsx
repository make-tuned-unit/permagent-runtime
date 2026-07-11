// PortalGodRays — volumetric-FEEL light from the Stargate event horizon.
//
// WORLD_VIEW_BIBLE.md §8: NOT real volumetrics (budget) — additive sprite geometry.
// A soft bloom-seed halo behind the ring + a slowly-turning corona of instanced
// additive shafts + a faint forward beam, so the portal reads as a source of light
// pouring into the rotunda, not a flat disc. depthWrite off, toneMapped off (bloom
// picks it up). One draw call for the 8-shaft corona (§8 instancing law). The scene
// is fill-rate bound (§0.6), so the additive area is kept deliberately modest.
// reduceMotion → a static corona (no rotation, constant opacity).

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { getReduceMotion } from '../../../../styles/tokens';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { unitPlane } from '../../props/geometries';

const HORIZON_COLOR = ENV.horizonBlue;
const RING_CENTER_Y = 3.5; // matches StargatePortal's inner ring group (RING_RADIUS+0.3)

const rayMat = new THREE.MeshBasicMaterial({
  color: HORIZON_COLOR,
  transparent: true,
  opacity: 0.06,
  depthWrite: false,
  side: THREE.DoubleSide,
  blending: THREE.AdditiveBlending,
  toneMapped: false,
});

const SHAFTS = 8;

function Corona({ reduceMotion }: { reduceMotion: boolean }) {
  const groupRef = useRef<THREE.Group>(null);

  useFrame((_, dt) => {
    if (reduceMotion) return;
    const g = groupRef.current;
    if (!g) return;
    g.rotation.z += dt * 0.06;
  });

  // Thin tall additive shafts fanning from the horizon center, in the ring plane.
  const shafts = useMemo<InstanceTransform[]>(
    () =>
      Array.from({ length: SHAFTS }, (_, i) => {
        const a = (i / SHAFTS) * Math.PI * 2;
        const len = 5 + (i % 2) * 2.2; // alternating shaft lengths
        // Plane is 1×1 facing +z; scale to a thin blade and offset outward so its
        // base sits at the horizon and it radiates out.
        return {
          position: [Math.cos(a) * (len / 2), Math.sin(a) * (len / 2), 0] as [number, number, number],
          rotation: [0, 0, a + Math.PI / 2] as [number, number, number],
          scale: [0.35, len, 1] as [number, number, number],
        };
      }),
    [],
  );

  return (
    <group ref={groupRef}>
      <InstancedProp name="portal.godrayShafts" geometry={unitPlane} material={rayMat} transforms={shafts} />
    </group>
  );
}

export function PortalGodRays() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const haloRef = useRef<THREE.Mesh>(null);

  useFrame(() => {
    if (reduceMotion) return;
    const m = haloRef.current;
    if (!m) return;
    (m.material as THREE.MeshBasicMaterial).opacity = 0.14 + 0.05 * Math.sin(performance.now() * 0.0006);
  });

  return (
    <group position-y={RING_CENTER_Y}>
      {/* Soft bloom-seed halo behind the ring. */}
      <mesh ref={haloRef} position-z={-0.3}>
        <circleGeometry args={[4.6, 40]} />
        <meshBasicMaterial
          color={HORIZON_COLOR}
          transparent
          opacity={0.14}
          depthWrite={false}
          side={THREE.DoubleSide}
          blending={THREE.AdditiveBlending}
          toneMapped={false}
        />
      </mesh>
      {/* Turning corona of light shafts. */}
      <Corona reduceMotion={reduceMotion} />
      {/* Faint forward beam pouring into the rotunda (toward the hall, +z). */}
      <mesh position-z={2.4} rotation-x={Math.PI / 2}>
        <coneGeometry args={[2.6, 5, 20, 1, true]} />
        <meshBasicMaterial
          color={HORIZON_COLOR}
          transparent
          opacity={0.05}
          depthWrite={false}
          side={THREE.DoubleSide}
          blending={THREE.AdditiveBlending}
          toneMapped={false}
        />
      </mesh>
    </group>
  );
}
