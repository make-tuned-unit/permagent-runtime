// Forum Antechamber blockout — WORLD_VIEW_BIBLE.md §3 A5 (tab: future Mesh).
// Entered through the relocated Stargate threshold. The cyber-classical
// language at its most austere: bare dark stone, densest fog, one cold
// downlight. Centerpiece: a dormant portal ring (no event horizon, chevrons
// unlit) above a plaque whose text comes ONLY from shared/meshStatus.ts —
// Mesh is offline today and the room renders that honestly (no fake activity).

import { useEffect, useMemo } from 'react';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { useMeshStatus } from '../../shared/meshStatus';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { ZoneShell, blockoutMat } from '../blockout';
import { TextPlate } from '../TextPlate';
import type { ZoneContentProps } from '../ZoneMount';

const CHEVRON_COUNT = 9;
const RING_R = 2.0;
const chevronGeo = new THREE.BoxGeometry(0.22, 0.32, 0.22);

function plaqueText(status: ReturnType<typeof useMeshStatus>): string {
  switch (status.state) {
    case 'offline':
      return 'MESH: NOT CONNECTED';
    case 'connecting':
      return 'MESH: CONNECTING';
    case 'connected':
      return `MESH: ${status.peerCount} PEER${status.peerCount === 1 ? '' : 'S'}`;
  }
}

// Dormant sibling of the Stargate: bare ring, unlit chevron stubs, no horizon.
function DormantRing() {
  const chevrons = useMemo<InstanceTransform[]>(
    () =>
      Array.from({ length: CHEVRON_COUNT }, (_, i) => {
        const a = (i / CHEVRON_COUNT) * Math.PI * 2 + Math.PI / 2;
        return {
          position: [29.2, 2.6 + Math.sin(a) * RING_R, Math.cos(a) * RING_R] as [number, number, number],
          rotation: [Math.PI / 2 - a, 0, 0] as [number, number, number],
        };
      }),
    []
  );

  return (
    <group>
      <group position={[29.2, 2.6, 0]} rotation-y={Math.PI / 2}>
        <mesh material={blockoutMat}>
          <torusGeometry args={[RING_R, 0.18, 10, 48]} />
        </mesh>
      </group>
      {/* Unlit chevron stubs — engraved Light-tier channels left dark */}
      <InstancedProp name="antechamber.dormantChevrons" geometry={chevronGeo} material={blockoutMat} transforms={chevrons} />
    </group>
  );
}

export default function AntechamberZone({ onReady }: ZoneContentProps) {
  useEffect(() => { onReady(); }, [onReady]);
  const status = useMeshStatus();

  return (
    <group>
      {/* Small liminal chamber, the most austere room in the world */}
      <ZoneShell name="antechamber.shell" cx={26.9} depth={10} width={10} height={6} causeway={{ fromX: 14.6, width: 4 }} />

      <DormantRing />

      {/* Engraved plaque — state comes ONLY from shared/meshStatus.ts */}
      <mesh position={[28.6, 1.05, 0]} rotation-y={Math.PI / 2} material={blockoutMat}>
        <boxGeometry args={[2.6, 0.9, 0.15]} />
      </mesh>
      <TextPlate
        text={plaqueText(status)}
        height={0.28}
        color={ENV.horizonBlue}
        opacity={status.state === 'offline' ? 0.45 : 0.85}
        position={[28.5, 1.05, 0]}
        rotation={[0, -Math.PI / 2, 0]}
      />

      {/* Single cold downlight */}
      <pointLight position={[26.9, 5.4, 0]} color={ENV.horizonBlue} intensity={0.5} distance={9} decay={2} />

      {/* Local haze — approximates the +0.004 fog-density bias (FogExp2 is
          scene-global in three.js); a dark back-face shell deepens the room */}
      <mesh position={[26.9, 3, 0]}>
        <boxGeometry args={[9.6, 5.8, 9.6]} />
        <meshBasicMaterial
          color={ENV.deepVoid}
          transparent
          opacity={0.22}
          side={THREE.BackSide}
          depthWrite={false}
        />
      </mesh>
    </group>
  );
}
