// Build wing blockout — WORLD_VIEW_BIBLE.md §3 A1 (tab: Build).
// Gray geometry at real scale (1u = 1m). Landmark: 6u gantry crane arm
// reaching over the threshold. Detail pass is gated on the owner's blockout review.
// Local frame: +x points away from hall center (ZoneMount applies the rotation).

import { useEffect, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { getReduceMotion } from '../../../../styles/tokens';
import { ZoneShell, blockoutMat } from '../blockout';
import type { ZoneContentProps } from '../ZoneMount';

const CRANE_ACCENT = ENV.neonAmber;

// 6u gantry crane arm silhouette: mast + jib over the threshold + hook.
function GantryCrane() {
  const hookRef = useRef<THREE.Group>(null);
  const reduceMotion = useRef(getReduceMotion());

  // Gentle hook sway — scalar math only, no per-frame allocations.
  useFrame(() => {
    if (reduceMotion.current || !hookRef.current) return;
    hookRef.current.rotation.z = 0.04 * Math.sin(performance.now() * 0.0006);
  });

  return (
    <group position={[26, 0, 3.5]}>
      {/* Mast */}
      <mesh position-y={3} material={blockoutMat} castShadow>
        <boxGeometry args={[0.7, 6, 0.7]} />
      </mesh>
      {/* Jib — reaches toward the threshold (-x) over the work floor */}
      <mesh position={[-3.2, 5.65, 0]} material={blockoutMat}>
        <boxGeometry args={[7.5, 0.5, 0.6]} />
      </mesh>
      {/* Counterweight */}
      <mesh position={[1.6, 5.4, 0]} material={blockoutMat}>
        <boxGeometry args={[1.2, 1.0, 1.0]} />
      </mesh>
      {/* Hook + cable */}
      <group ref={hookRef} position={[-6.2, 5.4, 0]}>
        <mesh position-y={-1.1} material={blockoutMat}>
          <boxGeometry args={[0.08, 2.2, 0.08]} />
        </mesh>
        <mesh position-y={-2.4} material={blockoutMat}>
          <boxGeometry args={[0.5, 0.5, 0.3]} />
        </mesh>
      </group>
      {/* The single light accent (silhouette rule §1): amber strip under the jib */}
      <mesh position={[-3.2, 5.36, 0]}>
        <boxGeometry args={[7.3, 0.06, 0.12]} />
        <meshBasicMaterial color={CRANE_ACCENT} transparent opacity={0.8} />
      </mesh>
    </group>
  );
}

export default function BuildZone({ onReady }: ZoneContentProps) {
  useEffect(() => { onReady(); }, [onReady]);

  return (
    <group>
      <ZoneShell name="build.shell" cx={24} depth={12} width={14} height={7} causeway={{ fromX: 15, width: 4.5 }} />
      <GantryCrane />
      {/* Bench row masses — stand-ins for W2's working bays */}
      <mesh position={[24, 0.45, -3]} material={blockoutMat} castShadow>
        <boxGeometry args={[8, 0.9, 1.4]} />
      </mesh>
      <mesh position={[24, 0.45, 0.5]} material={blockoutMat} castShadow>
        <boxGeometry args={[8, 0.9, 1.4]} />
      </mesh>
    </group>
  );
}
