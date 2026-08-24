// WatcherBeacon — the vigil tower on the platform's NE quarter.
//
// The Watcher's home (roster home sits at its base). A slender cyber-classical
// watchtower: stepped marble base, dark-stone shaft with an engraved trim
// channel in the Watcher's vigil steel-blue, a bronze cage, and a beacon orb
// that stays DARK until a REAL proactive_nudge arrives on /events — then it
// flares once (a 4s bloom) and the plaque beneath shows the real nudge
// subject/message for ~75s (agents/watcherNudge.ts is the single truth).
//
// HONESTY: no nudge this session ⇒ dark beacon, no plaque, nothing pulses.
// The tower never performs. LAW: zero per-frame allocations (material +
// scale mutation only); reduceMotion ⇒ the flare renders as a steady dim
// hold instead of the bloom curve.

import { useEffect, useMemo, useReducer, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { Html } from '@react-three/drei';
import * as THREE from 'three';
import { ENV, AGENT_TRIM } from '../shared/palette';
import { getReduceMotion } from '../../../styles/tokens';
import {
  NUDGE_PRESENT_MS,
  getNudge,
  resolveNudgeDisplay,
  useNudge,
} from '../agents/watcherNudge';

// Moved off the spiral-stair ground entry (2026-07-28): the old [8.6, -8.0]
// sat ~2u from stairPointAt(0) ≈ (10.6, -9.1) and physically blocked the
// climb. Now on the SW floor between the workbench station and the
// antechamber approach, clear of all walk targets.
export const BEACON_POS: [number, number, number] = [-5.5, 0, -10.5];
const TOWER_H = 4.6;

// Materials owned here (one-off prop; the beacon core is animated).
const towerStone = new THREE.MeshLambertMaterial({
  color: ENV.darkStone,
});
const towerMarble = new THREE.MeshLambertMaterial({
  color: ENV.marble,
});
const towerBronze = new THREE.MeshStandardMaterial({
  color: ENV.bronze,
  metalness: 0.75,
  roughness: 0.4,
});
const vigilTrim = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: AGENT_TRIM.watcher,
  emissiveIntensity: 0.5,
});
const beaconCore = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: AGENT_TRIM.watcher,
  emissiveIntensity: 0.12, // dark until a real nudge
});
const flareHalo = new THREE.MeshBasicMaterial({
  color: AGENT_TRIM.watcher,
  transparent: true,
  opacity: 0,
  depthWrite: false,
  blending: THREE.AdditiveBlending,
  toneMapped: false,
});

function NudgePlaque() {
  const nudge = useNudge();
  // Discrete re-renders only: on a new nudge (useNudge) and once when its
  // presentation window closes (a single scheduled re-render — no polling).
  const [, expire] = useReducer((n: number) => n + 1, 0);
  useEffect(() => {
    if (nudge.seq === 0) return;
    const remaining = nudge.at + NUDGE_PRESENT_MS - Date.now();
    if (remaining <= 0) return;
    const t = setTimeout(expire, remaining + 50);
    return () => clearTimeout(t);
  }, [nudge.seq, nudge.at]);

  const display = resolveNudgeDisplay(nudge, Date.now());
  if (!display.active || !nudge.subject) return null;
  return (
    <Html position={[0, TOWER_H + 1.5, 0]} center distanceFactor={16} style={{ pointerEvents: 'none' }}>
      <div
        style={{
          padding: '5px 12px',
          borderRadius: 4,
          background: 'rgba(10, 14, 26, 0.85)',
          border: `1px solid ${AGENT_TRIM.watcher}66`,
          boxShadow: `0 0 14px ${AGENT_TRIM.watcher}44`,
          color: '#E8E4DD',
          fontFamily: 'JetBrains Mono, monospace',
          fontSize: 10,
          letterSpacing: '0.12em',
          textAlign: 'center',
          maxWidth: 220,
        }}
      >
        <div style={{ color: AGENT_TRIM.watcher, fontWeight: 700 }}>THE WATCHER STIRS</div>
        <div style={{ marginTop: 2, opacity: 0.85 }}>{nudge.subject}</div>
        {nudge.message && (
          <div
            style={{
              marginTop: 2,
              opacity: 0.6,
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              maxWidth: 210,
            }}
          >
            {nudge.message}
          </div>
        )}
      </div>
    </Html>
  );
}

export function WatcherBeacon() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const coreRef = useRef<THREE.Mesh>(null);
  const haloRef = useRef<THREE.Mesh>(null);

  useFrame(() => {
    const core = coreRef.current;
    const halo = haloRef.current;
    if (!core || !halo) return;
    const d = resolveNudgeDisplay(getNudge(), Date.now());
    // reduceMotion: a steady dim hold while the presentation window is open —
    // legible without the bloom animation.
    const flare = reduceMotion ? (d.active ? 0.35 : 0) : d.flare;
    (core.material as THREE.MeshLambertMaterial).emissiveIntensity =
      0.12 + flare * 2.2 + (d.active ? 0.5 : 0);
    (halo.material as THREE.MeshBasicMaterial).opacity = flare * 0.55;
    halo.scale.setScalar(1 + flare * 2.4);
  });

  return (
    <group position={BEACON_POS}>
      {/* Stepped base */}
      <mesh position-y={0.12} material={towerMarble} castShadow>
        <cylinderGeometry args={[0.85, 1.0, 0.24, 8]} />
      </mesh>
      <mesh position-y={0.34} material={towerStone}>
        <cylinderGeometry args={[0.62, 0.72, 0.22, 8]} />
      </mesh>
      {/* Shaft */}
      <mesh position-y={TOWER_H / 2 + 0.4} material={towerStone} castShadow>
        <cylinderGeometry args={[0.3, 0.44, TOWER_H - 0.6, 8]} />
      </mesh>
      {/* Engraved vigil channel up the shaft (Light tier, engraved — §1) */}
      <mesh position={[0.3, TOWER_H / 2 + 0.4, 0]} material={vigilTrim}>
        <boxGeometry args={[0.05, TOWER_H - 1.0, 0.05]} />
      </mesh>
      {/* Bronze cage: two rings framing the beacon */}
      <mesh position-y={TOWER_H + 0.05} rotation-x={Math.PI / 2} material={towerBronze}>
        <torusGeometry args={[0.34, 0.035, 6, 20]} />
      </mesh>
      <mesh position-y={TOWER_H + 0.55} rotation-x={Math.PI / 2} material={towerBronze}>
        <torusGeometry args={[0.22, 0.03, 6, 20]} />
      </mesh>
      {/* Cage uprights */}
      {[0, 1, 2, 3].map((i) => {
        const a = (i / 4) * Math.PI * 2;
        return (
          <mesh
            key={i}
            position={[Math.cos(a) * 0.28, TOWER_H + 0.3, Math.sin(a) * 0.28]}
            material={towerBronze}
          >
            <cylinderGeometry args={[0.02, 0.02, 0.52, 6]} />
          </mesh>
        );
      })}
      {/* Cap */}
      <mesh position-y={TOWER_H + 0.72} material={towerStone}>
        <coneGeometry args={[0.3, 0.3, 8]} />
      </mesh>
      {/* The beacon orb — dark until a REAL nudge */}
      <mesh ref={coreRef} position-y={TOWER_H + 0.3} material={beaconCore}>
        <sphereGeometry args={[0.17, 14, 12]} />
      </mesh>
      {/* Flare halo (additive, hidden at rest) */}
      <mesh ref={haloRef} position-y={TOWER_H + 0.3} material={flareHalo}>
        <sphereGeometry args={[0.3, 14, 12]} />
      </mesh>
      <NudgePlaque />
    </group>
  );
}
