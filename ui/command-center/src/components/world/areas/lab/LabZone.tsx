// Lab blockout — WORLD_VIEW_BIBLE.md §3 A3 (tab: Skills/Trace; label "Lab").
// Absorbs the old observatory corner. Landmark: 5u experiment orrery suspended
// in the threshold sightline — the existing ArmillarySphere promoted/enlarged
// (rings + central orb, slow two-axis rotation, cyan accent for the Lab mood).

import { useEffect, useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { getReduceMotion } from '../../../../styles/tokens';
import { ZoneShell, blockoutMat } from '../blockout';
import type { ZoneContentProps } from '../ZoneMount';

// 5u orrery — enlarged ArmillarySphere, suspended from the ceiling.
function ExperimentOrrery() {
  const ref = useRef<THREE.Group>(null);
  const reduceMotion = useRef(getReduceMotion());

  // Same motion as the original ArmillarySphere (WorldFurniture), no allocations.
  useFrame((_, delta) => {
    if (reduceMotion.current || !ref.current) return;
    ref.current.rotation.y += delta * 0.2;
    ref.current.rotation.z += delta * 0.1;
  });

  const orbMat = useMemo(
    () => new THREE.MeshBasicMaterial({ color: ENV.neonCyan, transparent: true, opacity: 0.7 }),
    []
  );

  return (
    <group position={[24, 0, 0]}>
      {/* Suspension rod from the ceiling */}
      <mesh position-y={6.1} material={blockoutMat}>
        <cylinderGeometry args={[0.05, 0.05, 2.6, 6]} />
      </mesh>
      <group ref={ref} position-y={3.6}>
        <mesh material={blockoutMat}>
          <torusGeometry args={[2.4, 0.08, 8, 48]} />
        </mesh>
        <mesh rotation-x={Math.PI / 3} material={blockoutMat}>
          <torusGeometry args={[2.0, 0.08, 8, 48]} />
        </mesh>
        <mesh rotation-x={-Math.PI / 4} rotation-y={Math.PI / 4} material={blockoutMat}>
          <torusGeometry args={[1.6, 0.08, 8, 48]} />
        </mesh>
        {/* Central orb — the single cyan accent */}
        <mesh material={orbMat}>
          <sphereGeometry args={[0.4, 16, 16]} />
        </mesh>
        <pointLight color={ENV.neonCyan} intensity={0.4} distance={6} decay={2} />
      </group>
    </group>
  );
}

export default function LabZone({ onReady }: ZoneContentProps) {
  useEffect(() => { onReady(); }, [onReady]);

  return (
    <group>
      <ZoneShell name="lab.shell" cx={24} depth={12} width={14} height={7} causeway={{ fromX: 15, width: 4.5 }} />
      <ExperimentOrrery />
      {/* Instrument bench masses + trace-wall stand-in (W2 details later) */}
      <mesh position={[24, 0.45, -4.5]} material={blockoutMat} castShadow>
        <boxGeometry args={[7, 0.9, 1.2]} />
      </mesh>
      <mesh position={[24, 0.45, 4.5]} material={blockoutMat} castShadow>
        <boxGeometry args={[7, 0.9, 1.2]} />
      </mesh>
      {/* Trace wall — tall slab against the back wall */}
      <mesh position={[29.2, 2.8, 0]} material={blockoutMat}>
        <boxGeometry args={[0.3, 4.5, 6]} />
      </mesh>
    </group>
  );
}
