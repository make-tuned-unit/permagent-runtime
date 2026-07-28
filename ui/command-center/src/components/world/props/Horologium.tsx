// Horologium — the scheduler made physical (bible §3 A4's landmark, realized
// in the rotunda now that the satellite rooms are gone).
//
// A freestanding wheel of concentric bronze rings behind the Automate
// pedestal. Every tick-light on the face is one REAL scheduled job from
// /api/runs (agents/runsSignals.ts is the honesty boundary):
//
//   • amber tick     — that job is genuinely running right now (#694 status)
//   • dim cyan tick  — armed and idle
//   • gray tick      — paused
//   • red tick       — its last run errored or was missed
//
// While ANY job is really running the inner escapement ring turns; otherwise
// the wheel stands still. The chip below names the running job (its real
// source name). No scheduled jobs ⇒ a bare, still wheel — honest.
//
// BUDGET: 4 status buckets × InstancedProp + 4 one-off meshes ≈ 8 draw calls,
// no lights. LAW: zero per-frame allocations (tick transforms rebuilt only on
// poll changes via React state; the frame loop only rotates one group).
// reduceMotion ⇒ the escapement holds still even while running (status colors
// still tell the truth).

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { Html } from '@react-three/drei';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { getReduceMotion } from '../../../styles/tokens';
import { navigateToTool } from '../../../lib/store';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox } from './geometries';
import {
  metalBronze,
  stoneDark,
  stoneMarble,
  lightAmberTick,
  lightCyan,
  lightIdleTick,
  lightErrorTick,
} from './materials';
import { horologiumTicks, useRuns, type HorologiumTick } from '../agents/runsSignals';

// Placement: against the west colonnade, offset off the column at angle π so
// the wheel fills the bay between columns and faces the hall center.
const WHEEL_ANGLE = Math.PI * 0.93;
const WHEEL_R = 13.0;
const FACE_Y = 3.1; // wheel hub height
const OUTER_R = 2.05;
const TICK_R = 1.78;

function bucketTransforms(ticks: HorologiumTick[], status: HorologiumTick['status'][]): InstanceTransform[] {
  return ticks
    .filter((t) => status.includes(t.status))
    .map((t) => {
      // Face-local: angle 0 = top, clockwise. The face lies in the local XY plane.
      const x = Math.sin(t.angle) * TICK_R;
      const y = Math.cos(t.angle) * TICK_R;
      return {
        position: [x, y, 0.09] as [number, number, number],
        rotation: [0, 0, -t.angle] as [number, number, number],
        scale: [0.09, 0.34, 0.07] as [number, number, number],
      };
    });
}

export function Horologium() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const { schedules, loaded } = useRuns();
  const model = useMemo(() => horologiumTicks(schedules), [schedules]);

  const running = useMemo(() => bucketTransforms(model.ticks, ['working']), [model]);
  const armed = useMemo(() => bucketTransforms(model.ticks, ['idle']), [model]);
  const paused = useMemo(() => bucketTransforms(model.ticks, ['paused']), [model]);
  const failed = useMemo(() => bucketTransforms(model.ticks, ['error', 'missed']), [model]);

  const escapementRef = useRef<THREE.Group>(null);
  const anyRunningRef = useRef(false);
  anyRunningRef.current = model.anyRunning;

  useFrame((_, dt) => {
    if (reduceMotion) return;
    const g = escapementRef.current;
    if (!g) return;
    // The escapement turns ONLY while a schedule is really running.
    if (anyRunningRef.current) g.rotation.z -= dt * 0.35;
  });

  const px = Math.cos(WHEEL_ANGLE) * WHEEL_R;
  const pz = Math.sin(WHEEL_ANGLE) * WHEEL_R;

  return (
    // Click the wheel to open Automate — the tab that owns /api/runs schedules.
    <group
      position={[px, 0, pz]}
      rotation-y={Math.PI / 2 - WHEEL_ANGLE + Math.PI}
      onClick={(e) => { e.stopPropagation(); navigateToTool('automate'); }}
      onPointerOver={(e) => { e.stopPropagation(); document.body.style.cursor = 'pointer'; }}
      onPointerOut={(e) => { e.stopPropagation(); document.body.style.cursor = 'auto'; }}
    >
      {/* Plinth */}
      <mesh position-y={0.15} material={stoneMarble} castShadow>
        <boxGeometry args={[2.4, 0.3, 1.0]} />
      </mesh>
      <mesh position-y={0.55} material={stoneDark}>
        <boxGeometry args={[1.4, 0.5, 0.7]} />
      </mesh>
      {/* Support column up to the hub */}
      <mesh position-y={1.7} material={stoneDark}>
        <cylinderGeometry args={[0.16, 0.24, 2.0, 8]} />
      </mesh>

      {/* The wheel face group (local XY plane, +z toward the hall) */}
      <group position-y={FACE_Y}>
        {/* Outer bronze ring */}
        <mesh material={metalBronze} castShadow>
          <torusGeometry args={[OUTER_R, 0.09, 10, 48]} />
        </mesh>
        {/* Tick track ring — engraved cyan hairline */}
        <mesh position-z={0.04} material={lightCyan}>
          <torusGeometry args={[TICK_R, 0.015, 6, 48]} />
        </mesh>
        {/* Inner escapement ring — turns only while a job really runs */}
        <group ref={escapementRef}>
          <mesh material={metalBronze}>
            <torusGeometry args={[1.15, 0.055, 8, 40]} />
          </mesh>
          {/* Escapement spokes (part of the turning read) */}
          {[0, 1, 2].map((i) => (
            <mesh key={i} rotation-z={(i / 3) * Math.PI} material={metalBronze}>
              <boxGeometry args={[2.16, 0.05, 0.05]} />
            </mesh>
          ))}
        </group>
        {/* Hub */}
        <mesh rotation-x={Math.PI / 2} material={stoneDark}>
          <cylinderGeometry args={[0.22, 0.22, 0.18, 12]} />
        </mesh>

        {/* REAL schedule ticks, bucketed by live status */}
        {running.length > 0 && (
          <InstancedProp name="horologium.running" geometry={unitBox} material={lightAmberTick} transforms={running} />
        )}
        {armed.length > 0 && (
          <InstancedProp name="horologium.armed" geometry={unitBox} material={lightCyan} transforms={armed} />
        )}
        {paused.length > 0 && (
          <InstancedProp name="horologium.paused" geometry={unitBox} material={lightIdleTick} transforms={paused} />
        )}
        {failed.length > 0 && (
          <InstancedProp name="horologium.failed" geometry={unitBox} material={lightErrorTick} transforms={failed} />
        )}
      </group>

      {/* Honest chip: the running job's real name, or the armed count */}
      {loaded && (model.runningName || model.ticks.length > 0) && (
        <Html position={[0, FACE_Y - OUTER_R - 0.55, 0.4]} center distanceFactor={15} style={{ pointerEvents: 'none' }}>
          <div
            style={{
              padding: '4px 10px',
              borderRadius: 4,
              background: 'rgba(10, 14, 26, 0.82)',
              border: `1px solid ${model.runningName ? ENV.neonAmber : ENV.bronze}55`,
              boxShadow: model.runningName ? `0 0 12px ${ENV.neonAmber}33` : 'none',
              color: '#E8E4DD',
              fontFamily: 'JetBrains Mono, monospace',
              fontSize: 10,
              letterSpacing: '0.14em',
              whiteSpace: 'nowrap',
              textAlign: 'center',
            }}
          >
            {model.runningName ? (
              <span style={{ color: ENV.neonAmber }}>⚙ RUNNING · {model.runningName}</span>
            ) : (
              <span style={{ opacity: 0.7 }}>
                {model.ticks.length} SCHEDULE{model.ticks.length === 1 ? '' : 'S'} ARMED
                {model.overflow > 0 ? ` · +${model.overflow}` : ''}
              </span>
            )}
          </div>
        </Html>
      )}
    </group>
  );
}
