// PetitionBasin — the Decision Inbox standing in the hall.
//
// A stone basin on the rotunda's south-east floor. Every pending decision in
// the REAL inbox (/api/decisions, via agents/decisionSignals.ts) is a sealed
// scroll hovering over the water: a LIVE decision_created drops a new scroll
// in from above; answering one sends its scroll ascending out of the world.
// The chip counts the real total. Zero pending ⇒ an empty, still basin.
//
// Scroll color law: pending decisions are work waiting on the USER — cyan
// (HUD "available/ready for you"), never amber (amber is an agent working).
//
// BUDGET: basin 3 meshes + one instanced scroll body + one instanced bronze
// seal band ≈ 5 draw calls, no lights. LAW: zero per-frame allocations — the
// per-frame loop only writes instance matrices for ≤ 8 scrolls through one
// scratch Object3D. reduceMotion ⇒ scrolls hold still (no bob; arrivals and
// ascents render at their settled positions).

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { Html } from '@react-three/drei';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { getReduceMotion } from '../../../styles/tokens';
import { stoneDark, stoneMarble, metalBronze, lightCyan } from './materials';
import {
  ASCEND_MS,
  PETITION_CAP,
  getPetitions,
  sweepPetitions,
  usePetitions,
} from '../agents/decisionSignals';

export const BASIN_POS: [number, number, number] = [5.2, 0, 4.6];
const HOVER_Y = 1.55;
const DROP_MS = 900;

// Scroll slots ring the basin center (deterministic — index-stable).
const SLOTS: [number, number][] = Array.from({ length: PETITION_CAP }, (_, i) => {
  const a = (i / PETITION_CAP) * Math.PI * 2;
  return [Math.cos(a) * 0.62, Math.sin(a) * 0.62];
});

const scrollGeo = new THREE.CylinderGeometry(0.05, 0.05, 0.42, 8);
const bandGeo = new THREE.TorusGeometry(0.055, 0.016, 6, 12);

const _obj = new THREE.Object3D();

function easeOutCubic(t: number): number {
  return 1 - Math.pow(1 - t, 3);
}

function ScrollField({ reduceMotion }: { reduceMotion: boolean }) {
  const bodyRef = useRef<THREE.InstancedMesh>(null);
  const bandRef = useRef<THREE.InstancedMesh>(null);

  useFrame(({ clock }) => {
    const bodies = bodyRef.current;
    const bands = bandRef.current;
    if (!bodies || !bands) return;
    const now = Date.now();
    // Drop scrolls whose ascend finished (publishes only on change).
    sweepPetitions(now);
    const { petitions } = getPetitions();
    const t = clock.elapsedTime;
    let n = 0;
    for (let i = 0; i < petitions.length && n < PETITION_CAP; i++) {
      const p = petitions[i];
      const [sx, sz] = SLOTS[n % PETITION_CAP];
      // Arrival: drop in from above over DROP_MS. Resolution: ascend + shrink.
      let y = HOVER_Y;
      let scale = 1;
      const sinceSeen = now - p.seenAt;
      if (!reduceMotion && sinceSeen < DROP_MS) {
        const k = easeOutCubic(Math.min(1, sinceSeen / DROP_MS));
        y = HOVER_Y + (1 - k) * 2.6;
      }
      if (p.resolvedAt) {
        const k = Math.min(1, (now - p.resolvedAt) / ASCEND_MS);
        if (reduceMotion) {
          scale = 1 - k; // no travel — just leave
        } else {
          y = HOVER_Y + easeOutCubic(k) * 5.5;
          scale = 1 - k * 0.85;
        }
      } else if (!reduceMotion) {
        // Idle bob + slow turn — waiting, patient.
        y += 0.06 * Math.sin(t * 1.1 + n * 1.7);
      }
      _obj.position.set(sx, y, sz);
      _obj.rotation.set(0, reduceMotion ? 0 : t * 0.25 + n, 0);
      _obj.scale.setScalar(Math.max(0.0001, scale));
      _obj.updateMatrix();
      bodies.setMatrixAt(n, _obj.matrix);
      // Seal band rides the scroll's waist.
      _obj.position.y = y;
      _obj.rotation.set(Math.PI / 2, 0, reduceMotion ? 0 : t * 0.25 + n);
      _obj.updateMatrix();
      bands.setMatrixAt(n, _obj.matrix);
      n++;
    }
    bodies.count = n;
    bands.count = n;
    bodies.instanceMatrix.needsUpdate = true;
    bands.instanceMatrix.needsUpdate = true;
  });

  return (
    <group>
      <instancedMesh ref={bodyRef} args={[scrollGeo, lightCyan, PETITION_CAP]} />
      <instancedMesh ref={bandRef} args={[bandGeo, metalBronze, PETITION_CAP]} />
    </group>
  );
}

export function PetitionBasin() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const { petitions, totalPending, loaded } = usePetitions();
  const open = petitions.filter((p) => !p.resolvedAt).length;

  return (
    <group position={BASIN_POS}>
      {/* Basin: stepped bowl */}
      <mesh position-y={0.12} material={stoneMarble} castShadow>
        <cylinderGeometry args={[1.05, 1.2, 0.24, 10]} />
      </mesh>
      <mesh position-y={0.34} material={stoneDark}>
        <cylinderGeometry args={[0.92, 1.0, 0.24, 10]} />
      </mesh>
      {/* Bowl water — still, dark; the §1 sanctioned holo/transparency tier */}
      <mesh position-y={0.47} rotation-x={-Math.PI / 2}>
        <circleGeometry args={[0.84, 24]} />
        <meshStandardMaterial color="#16323E" roughness={0.12} metalness={0.5} transparent opacity={0.9} />
      </mesh>
      {/* Rim hairline — dim when empty, present when petitions wait */}
      <mesh position-y={0.48} rotation-x={-Math.PI / 2}>
        <ringGeometry args={[0.86, 0.92, 24]} />
        <meshBasicMaterial
          color={ENV.neonCyan}
          transparent
          opacity={open > 0 ? 0.4 : 0.08}
          depthWrite={false}
        />
      </mesh>

      <ScrollField reduceMotion={reduceMotion} />

      {/* Honest count chip — only when something really waits */}
      {loaded && totalPending > 0 && (
        <Html position={[0, 2.6, 0]} center distanceFactor={14} style={{ pointerEvents: 'none' }}>
          <div
            style={{
              padding: '4px 10px',
              borderRadius: 4,
              background: 'rgba(10, 14, 26, 0.82)',
              border: `1px solid ${ENV.neonCyan}55`,
              boxShadow: `0 0 12px ${ENV.neonCyan}33`,
              color: '#E8E4DD',
              fontFamily: 'JetBrains Mono, monospace',
              fontSize: 9,
              letterSpacing: '0.16em',
              whiteSpace: 'nowrap',
            }}
          >
            <span style={{ color: ENV.neonCyan }}>
              {totalPending} DECISION{totalPending === 1 ? '' : 'S'} AWAIT YOU
            </span>
            {totalPending > PETITION_CAP && (
              <span style={{ opacity: 0.6 }}> · +{totalPending - PETITION_CAP} BEYOND THE BASIN</span>
            )}
          </div>
        </Html>
      )}
    </group>
  );
}
