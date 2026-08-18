// Brain Archive blockout — WORLD_VIEW_BIBLE.md §3 A2 (tab: Brain).
// The only violet-dominant zone. Landmark: 7u rotating violet memory core —
// a lathe-turned obelisk with orbiting index rings, visible down the south
// threshold. Gray blockout; detail pass gated on the owner's review.

import { useEffect, useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { getReduceMotion } from '../../../../styles/tokens';
import { ZoneShell, blockoutMat } from '../blockout';
import type { ZoneContentProps } from '../ZoneMount';

// 7u rotating memory-core obelisk with orbiting index rings.
function MemoryCore() {
  const coreRef = useRef<THREE.Group>(null);
  const ringsRef = useRef<THREE.Group>(null);
  const reduceMotion = useRef(getReduceMotion());

  useFrame((_, delta) => {
    if (reduceMotion.current) return;
    if (coreRef.current) coreRef.current.rotation.y += delta * 0.15;
    if (ringsRef.current) {
      ringsRef.current.rotation.y -= delta * 0.3;
    }
  });

  const violetMat = useMemo(
    () => new THREE.MeshBasicMaterial({ color: ENV.violet, transparent: true, opacity: 0.85 }),
    []
  );

  return (
    <group position={[25, 0, 0]}>
      {/* Plinth */}
      <mesh position-y={0.3} material={blockoutMat} castShadow>
        <cylinderGeometry args={[1.5, 1.8, 0.6, 8]} />
      </mesh>
      {/* Lathe-turned obelisk body — rotates slowly */}
      <group ref={coreRef}>
        <mesh position-y={3.4} material={blockoutMat} castShadow>
          <cylinderGeometry args={[0.45, 0.95, 5.6, 10]} />
        </mesh>
        {/* Tip */}
        <mesh position-y={6.55} material={blockoutMat}>
          <coneGeometry args={[0.45, 0.9, 10]} />
        </mesh>
        {/* The landmark accent: violet seam bands engraved in the core */}
        <mesh position-y={2.2} material={violetMat}>
          <torusGeometry args={[0.85, 0.025, 6, 32]} />
        </mesh>
        <mesh position-y={4.6} rotation-x={Math.PI / 2} material={violetMat}>
          <torusGeometry args={[0.62, 0.025, 6, 32]} />
        </mesh>
      </group>
      {/* Orbiting index rings */}
      <group ref={ringsRef} position-y={3.6}>
        <mesh rotation-x={Math.PI / 2 - 0.18} material={violetMat}>
          <torusGeometry args={[1.7, 0.035, 6, 48]} />
        </mesh>
        <mesh rotation-x={Math.PI / 2 + 0.24} position-y={1.1} material={violetMat}>
          <torusGeometry args={[1.3, 0.03, 6, 48]} />
        </mesh>
      </group>
    </group>
  );
}

export default function BrainZone({ onReady }: ZoneContentProps) {
  useEffect(() => { onReady(); }, [onReady]);

  return (
    <group>
      <ZoneShell name="brain.shell" cx={24} depth={12} width={14} height={8} causeway={{ fromX: 15, width: 4.5 }} />
      <MemoryCore />
      {/* Shelf-canyon masses — stand-ins for W2's instanced deep stacks */}
      <mesh position={[24, 1.75, -5]} material={blockoutMat} castShadow>
        <boxGeometry args={[9, 3.5, 1]} />
      </mesh>
      <mesh position={[24, 1.75, 5]} material={blockoutMat} castShadow>
        <boxGeometry args={[9, 3.5, 1]} />
      </mesh>
    </group>
  );
}
