// The task dais — the rotunda's central platform. When an agent picks up real
// work (HUD state → working), behavior walks it onto this platform and fires
// the dais beam (agents/daisBus): a column of light from the oculus transmits
// the work DOWN into the agent — descending rings + impact glow — then the
// agent steps off and engages at its seat. The beam only ever plays on that
// real trigger; idle state is a quiet stone platform with a soft edge ring.

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { makeStoneTexture } from '../../shared/stoneTexture';
import { DAIS, BEAM_MS, getDaisBeam, getDaisPresence, triggerDaisBeam } from '../../agents/daisBus';
import { DOME_HEIGHT } from '../../constants';

const BEAM_TOP = DOME_HEIGHT + 4; // reads as descending from the oculus
const RING_COUNT = 4;

// Dev hook for the screenshot harness (worldcensus page): lets a headless run
// fire the beam without waiting for a real working transition.
if (typeof window !== 'undefined' && import.meta.env.DEV) {
  (window as unknown as Record<string, unknown>).__triggerDaisBeam = triggerDaisBeam;
}

export function TaskDais() {
  const beamRef = useRef<THREE.Group>(null);
  const coreRef = useRef<THREE.Mesh>(null);
  const impactRef = useRef<THREE.Mesh>(null);
  const ringRefs = useRef<(THREE.Mesh | null)[]>([]);
  const edgeRef = useRef<THREE.Mesh>(null);
  const seenSeq = useRef(0);

  const platformMat = useMemo(() => {
    const map = makeStoneTexture('#a4aab8');
    return new THREE.MeshLambertMaterial({ map });
  }, []);
  const stepMat = useMemo(
    () => new THREE.MeshLambertMaterial({ map: makeStoneTexture('#8e94a2') }),
    [],
  );

  useFrame(() => {
    const beam = getDaisBeam();
    const g = beamRef.current;
    if (!g) return;
    if (beam.seq !== seenSeq.current) seenSeq.current = beam.seq;

    const elapsed = beam.seq > 0 ? performance.now() - beam.startedAt : Infinity;
    const taskActive = elapsed < BEAM_MS;
    // Sustained presence: Henry stands here for the whole open conversation —
    // a continuous soft column, gentler than the task pulse.
    const presence = getDaisPresence();
    const active = taskActive || presence;
    g.visible = active;

    // Edge ring: quiet breathing normally, surges while the beam plays.
    if (edgeRef.current) {
      const mat = edgeRef.current.material as THREE.MeshBasicMaterial;
      const breathe = 0.22 + 0.08 * Math.sin(performance.now() * 0.0012);
      mat.opacity = active ? (taskActive ? 0.75 : 0.5) : breathe;
    }
    if (!active) return;

    // Task pulse: 0..1 envelope. Presence: steady loop at reduced strength.
    const t = taskActive ? elapsed / BEAM_MS : (performance.now() / 4000) % 1;
    const env = taskActive
      ? t < 0.12 ? t / 0.12 : t > 0.82 ? (1 - t) / 0.18 : 1
      : 0.55;

    if (coreRef.current) {
      const mat = coreRef.current.material as THREE.MeshBasicMaterial;
      mat.opacity = 0.34 * env;
      // Subtle shimmer so the column reads as energy, not glass.
      coreRef.current.scale.x = coreRef.current.scale.z =
        1 + 0.06 * Math.sin(performance.now() * 0.02);
    }
    // Descending payload rings: staggered, each travels top → platform.
    for (let i = 0; i < RING_COUNT; i++) {
      const ring = ringRefs.current[i];
      if (!ring) continue;
      const phase = (t * 1.6 + i / RING_COUNT) % 1;
      ring.position.y = BEAM_TOP - phase * (BEAM_TOP - DAIS.topY - 0.2);
      const mat = ring.material as THREE.MeshBasicMaterial;
      mat.opacity = 0.55 * env * Math.sin(phase * Math.PI);
      ring.scale.setScalar(0.7 + phase * 0.5);
    }
    if (impactRef.current) {
      const mat = impactRef.current.material as THREE.MeshBasicMaterial;
      mat.opacity = 0.5 * env * (0.7 + 0.3 * Math.sin(performance.now() * 0.015));
      impactRef.current.scale.setScalar(1 + 0.12 * Math.sin(performance.now() * 0.01));
    }
  });

  return (
    <group position={[DAIS.x, 0, DAIS.z]}>
      {/* Step ring — the low rim that reads "step up here". */}
      <mesh position-y={0.07} material={stepMat} receiveShadow>
        <cylinderGeometry args={[DAIS.radius + 0.65, DAIS.radius + 0.85, 0.14, 48]} />
      </mesh>
      {/* The platform itself. */}
      <mesh position-y={DAIS.topY / 2 + 0.02} material={platformMat} castShadow receiveShadow>
        <cylinderGeometry args={[DAIS.radius, DAIS.radius + 0.25, DAIS.topY, 48]} />
      </mesh>
      {/* Soft cyan edge ring on the platform lip. */}
      <mesh ref={edgeRef} rotation-x={-Math.PI / 2} position-y={DAIS.topY + 0.02}>
        <ringGeometry args={[DAIS.radius - 0.18, DAIS.radius - 0.04, 48]} />
        <meshBasicMaterial
          color={ENV.neonCyan}
          transparent
          opacity={0.25}
          depthWrite={false}
          blending={THREE.AdditiveBlending}
          toneMapped={false}
        />
      </mesh>

      {/* The work-transmission beam (hidden until a real trigger). */}
      <group ref={beamRef} visible={false}>
        {/* Core column, oculus → platform. */}
        <mesh
          ref={coreRef}
          position-y={(BEAM_TOP + DAIS.topY) / 2}
        >
          <cylinderGeometry args={[0.55, 0.32, BEAM_TOP - DAIS.topY, 20, 1, true]} />
          <meshBasicMaterial
            color={ENV.neonCyan}
            transparent
            opacity={0}
            depthWrite={false}
            side={THREE.DoubleSide}
            blending={THREE.AdditiveBlending}
            toneMapped={false}
          />
        </mesh>
        {/* Descending payload rings — the "work transmitted downward". */}
        {Array.from({ length: RING_COUNT }, (_, i) => (
          <mesh
            key={i}
            ref={(el) => { ringRefs.current[i] = el; }}
            rotation-x={-Math.PI / 2}
          >
            <torusGeometry args={[0.5, 0.045, 6, 24]} />
            <meshBasicMaterial
              color="#9be8ff"
              transparent
              opacity={0}
              depthWrite={false}
              blending={THREE.AdditiveBlending}
              toneMapped={false}
            />
          </mesh>
        ))}
        {/* Impact glow on the platform surface. */}
        <mesh ref={impactRef} rotation-x={-Math.PI / 2} position-y={DAIS.topY + 0.04}>
          <ringGeometry args={[0.2, 1.1, 32]} />
          <meshBasicMaterial
            color={ENV.neonCyan}
            transparent
            opacity={0}
            depthWrite={false}
            blending={THREE.AdditiveBlending}
            toneMapped={false}
          />
        </mesh>
      </group>
    </group>
  );
}
