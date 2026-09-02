// Automate hall — WORLD_VIEW_BIBLE.md §3 A4 (tab: Automate).
// Narrow gallery of scheduler boards. Landmark: wall-mounted 6u horologium —
// concentric bronze rings with cyan tick lights pulsing on a slow clock.
//
// The gallery itself is no longer a blockout (agent-QA D19): the three stone
// masses that stood in for scheduler steles are gone, replaced by one stele per
// REAL registered job, coloured by that job's real last outcome and pulsing
// only while a run is genuinely in flight (`areas/automate/scheduleActivity` →
// `props/AutomateSteles`). Nothing here needed a new event: `schedule_changed`
// has fired from nine call sites since 2026-08-25 and no world module listened.
//
// The horologium stays a clock. A clock marking time is true whatever the
// scheduler is doing, so its slow pulse is not a claim about work — the steles
// are where this room says what is happening.

import { useEffect, useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { getReduceMotion } from '../../../../styles/tokens';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { ZoneShell } from '../blockout';
import type { ZoneContentProps } from '../ZoneMount';
import { AutomateSteles } from '../../props/AutomateSteles';
import { useScheduleActivity } from './scheduleActivity';

// Module-level so the anchor registry sees a stable identity: a fresh array
// literal each render would re-register the gallery's seats every frame.
const STELE_ORIGIN: [number, number, number] = [24.5, 0, 0];

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
  // Real scheduler state. Until a source answers, `jobs` is empty and the
  // gallery renders as an empty gallery — an unknown scheduler must not read
  // as a healthy one.
  const { jobs } = useScheduleActivity();

  return (
    <group>
      {/* Narrow gallery per §3 — tighter width than the other wings */}
      <ZoneShell name="automate.shell" cx={24} depth={12} width={10} height={7} causeway={{ fromX: 15, width: 4.5 }} />
      <Horologium />
      {/* The steles face the causeway (-x), so the room reads on approach. */}
      <AutomateSteles jobs={jobs} position={STELE_ORIGIN} rotationY={-Math.PI / 2} />
    </group>
  );
}
