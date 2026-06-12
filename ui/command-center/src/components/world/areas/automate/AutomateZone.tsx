// Automate hall blockout — WORLD_VIEW_BIBLE.md §3 A4 (tab: Automate).
// Narrow gallery of scheduler boards. Landmark: wall-mounted 6u horologium —
// concentric bronze rings with cyan tick lights pulsing on a slow clock
// (the only animation in the room).

import { useEffect, useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { getReduceMotion } from '../../../../styles/tokens';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { ZoneShell, blockoutMat } from '../blockout';
import type { ZoneContentProps } from '../ZoneMount';

const TICK_COUNT = 12;
const tickGeo = new THREE.BoxGeometry(0.1, 0.22, 0.1);

// Wall-mounted 6u horologium on the back wall, facing the threshold (-x).
function Horologium() {
  const reduceMotion = useRef(getReduceMotion());

  const tickMat = useMemo(
    () => new THREE.MeshBasicMaterial({ color: ENV.neonCyan, transparent: true, opacity: 0.8 }),
    []
  );

  // Slow clock pulse on the shared tick material — no per-frame allocations.
  useFrame(() => {
    if (reduceMotion.current) return;
    tickMat.opacity = 0.45 + 0.35 * Math.sin(performance.now() * 0.0005);
  });

  const ringMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: ENV.bronze, roughness: 0.45, metalness: 0.65 }),
    []
  );

  // Ticks around the outer ring, in the wall plane (local YZ).
  const ticks = useMemo<InstanceTransform[]>(
    () =>
      Array.from({ length: TICK_COUNT }, (_, i) => {
        const a = (i / TICK_COUNT) * Math.PI * 2;
        return {
          position: [29.3, 4 + Math.sin(a) * 3.1, Math.cos(a) * 3.1] as [number, number, number],
          rotation: [Math.PI / 2 - a, 0, 0] as [number, number, number],
        };
      }),
    []
  );

  return (
    <group>
      {/* Concentric rings mounted just off the back wall, facing -x */}
      <group position={[29.4, 4, 0]} rotation-y={Math.PI / 2}>
        <mesh material={ringMat}>
          <torusGeometry args={[3.0, 0.1, 8, 48]} />
        </mesh>
        <mesh material={ringMat}>
          <torusGeometry args={[2.2, 0.08, 8, 40]} />
        </mesh>
        <mesh material={ringMat}>
          <torusGeometry args={[1.4, 0.06, 8, 32]} />
        </mesh>
      </group>
      {/* Cyan tick lights — instanced (1 draw call) */}
      <InstancedProp name="automate.horologiumTicks" geometry={tickGeo} material={tickMat} transforms={ticks} />
    </group>
  );
}

export default function AutomateZone({ onReady }: ZoneContentProps) {
  useEffect(() => { onReady(); }, [onReady]);

  return (
    <group>
      {/* Narrow gallery per §3 — tighter width than the other wings */}
      <ZoneShell name="automate.shell" cx={24} depth={12} width={10} height={7} causeway={{ fromX: 15, width: 4.5 }} />
      <Horologium />
      {/* Scheduler stele masses + long planning table stand-ins */}
      <mesh position={[23, 1.6, -3.8]} material={blockoutMat} castShadow>
        <boxGeometry args={[6, 3.2, 0.5]} />
      </mesh>
      <mesh position={[23, 1.6, 3.8]} material={blockoutMat} castShadow>
        <boxGeometry args={[6, 3.2, 0.5]} />
      </mesh>
      <mesh position={[24, 0.45, 0]} material={blockoutMat} castShadow>
        <boxGeometry args={[7, 0.9, 1.6]} />
      </mesh>
    </group>
  );
}
