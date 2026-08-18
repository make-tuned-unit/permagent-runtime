// Atmosphere — WATER + GROWTH.
//
// Water + growing things represent the user: the spring rises on the Brain's
// first memory, ingestion is rainfall, recall is the river running. Trees grow
// where memory pools. The agents build around water, never dam it.
//
// HONESTY: water IS the user, so its PRESENCE is bound to a REAL signal — the
// Brain's memory count (atmosphere/worldSignals → /api/brain/graph).
//   • memoryCount === 0  → the spring is DORMANT: dry stone, no flow, no pools.
//     An honest render of a never-used Brain. We do NOT fake a trickle.
//   • memoryCount  >  0  → the spring has risen (the first memory). Flow strength
//     and pool extent scale with the count (a coarse, log-ish map).
//   • flow level (recall-as-river + rainfall) modulates river brightness/ripple,
//     event-driven from real /events. No recall_* event exists yet — the river
//     currently runs on describe/task activity.
//
// The spring + channel + crown pools render at rotunda-floor level.
//
// LAW: zero per-frame allocations; reduceMotion → static water (no ripple, no
// flow animation); InstancedMesh for the repeated pool/grove props.

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { ROTUNDA_RADIUS } from '../constants';
import { getReduceMotion } from '../../../styles/tokens';
import { getWorldSignals, useWorldSignals } from './worldSignals';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';

// Water is cyber-classical cold cyan-violet — it reads as "the user's substrate".
// A light temperature, not a palette accent, so it lives here (cf. Atmosphere.tsx).
const WATER = {
  surface: '#1E5A6E',
  glow: ENV.neonCyan,
} as const;

// Geography at rotunda-floor level.
const SPRING_POS: [number, number, number] = [0, 0.06, ROTUNDA_RADIUS - 3.5];
// The river runs a floor channel from the spring toward the hall center.
const CHANNEL_FROM = new THREE.Vector3(0, 0.05, ROTUNDA_RADIUS - 3.5);
const CHANNEL_TO = new THREE.Vector3(0, 0.05, 2);

/** Coarse, slow map from real memory count → water "fullness" in [0,1]. */
export function fullnessFromMemories(count: number): number {
  if (count <= 0) return 0;
  // log-ish: 1 memory already reads; saturates around ~200 memories (months).
  return Math.min(1, Math.log10(count + 1) / Math.log10(200));
}

/**
 * The spring: a small still pool where water breaks through the stone. Its
 * radius and glow scale with fullness; at fullness 0 it does not render at all.
 * Ripple animation modulated by the event flow level; static under reduceMotion.
 */
function Spring({ reduceMotion }: { reduceMotion: boolean }) {
  const ref = useRef<THREE.Mesh>(null);
  const glowRef = useRef<THREE.Mesh>(null);

  useFrame(() => {
    const s = getWorldSignals();
    const fullness = fullnessFromMemories(s.memoryCount);
    const mesh = ref.current;
    const glow = glowRef.current;
    if (!mesh || !glow) return;
    // Presence: dormant Brain ⇒ invisible spring (honest dry stone).
    const present = fullness > 0;
    mesh.visible = present;
    glow.visible = present;
    if (!present) return;

    const r = 0.6 + 1.4 * fullness; // grows with the Brain
    mesh.scale.set(r, r, 1);
    glow.scale.set(r, r, 1);

    if (reduceMotion) {
      (glow.material as THREE.MeshBasicMaterial).opacity = 0.35;
      return;
    }
    // Recall running + rainfall make the spring shimmer brighter.
    const shimmer = 0.25 + 0.15 * Math.sin(performance.now() * 0.0012) + 0.25 * s.flow;
    (glow.material as THREE.MeshBasicMaterial).opacity = Math.min(0.7, shimmer);
  });

  return (
    <group position={SPRING_POS}>
      {/* Still pool surface */}
      <mesh ref={ref} rotation-x={-Math.PI / 2}>
        <circleGeometry args={[1, 40]} />
        <meshStandardMaterial
          color={WATER.surface}
          roughness={0.1}
          metalness={0.6}
          transparent
          opacity={0.85}
        />
      </mesh>
      {/* Cold rim glow where it breaks the stone */}
      <mesh ref={glowRef} rotation-x={-Math.PI / 2} position-y={0.01}>
        <ringGeometry args={[0.85, 1.05, 40]} />
        <meshBasicMaterial color={WATER.glow} transparent opacity={0.35} depthWrite={false} />
      </mesh>
    </group>
  );
}

/**
 * The river: a thin channel of running water from the spring toward the hall.
 * Brightness + a scrolling ripple read the event flow level (recall-as-river,
 * rainfall). Dormant (invisible) when there are no memories. Static under
 * reduceMotion (constant dim surface, no scroll).
 */
function River({ reduceMotion }: { reduceMotion: boolean }) {
  const ref = useRef<THREE.Mesh>(null);

  const { position, quaternion, length } = useMemo(() => {
    const mid = CHANNEL_FROM.clone().lerp(CHANNEL_TO, 0.5);
    const dir = CHANNEL_TO.clone().sub(CHANNEL_FROM);
    const len = dir.length();
    dir.normalize();
    // Plane default normal +z, lying flat we rotate -x; align its length (local x)
    // to the channel direction in the XZ plane.
    const angle = Math.atan2(dir.x, dir.z);
    const q = new THREE.Quaternion().setFromEuler(
      new THREE.Euler(-Math.PI / 2, angle, 0, 'YXZ')
    );
    return { position: mid, quaternion: q, length: len };
  }, []);

  useFrame(() => {
    const mesh = ref.current;
    if (!mesh) return;
    const s = getWorldSignals();
    const fullness = fullnessFromMemories(s.memoryCount);
    mesh.visible = fullness > 0;
    if (!mesh.visible) return;
    const mat = mesh.material as THREE.MeshBasicMaterial;
    if (reduceMotion) {
      mat.opacity = 0.2 + 0.25 * fullness;
      return;
    }
    // Running brighter while recall flows; gentle base shimmer otherwise.
    const base = 0.18 + 0.25 * fullness;
    mat.opacity = Math.min(0.75, base + 0.35 * s.flow + 0.04 * Math.sin(performance.now() * 0.002));
  });

  return (
    <mesh ref={ref} position={position.toArray()} quaternion={quaternion}>
      <planeGeometry args={[length, 0.7]} />
      <meshBasicMaterial color={WATER.glow} transparent opacity={0.2} depthWrite={false} />
    </mesh>
  );
}

/**
 * Crown pools + groves — groves at still pools, a canopy. A
 * few still pools ringing the hall, each with a small grove cone (young tree).
 * They appear progressively as the Brain matures (more memories → more groves),
 * answered nature framing the climb. Instanced (one draw call each). Static
 * geometry; their presence count is the only "animation" and it changes on the
 * slow graph poll, so no per-frame work and no reduceMotion branch needed.
 */
export const GROVE_SLOTS: { x: number; z: number }[] = [
  // Slot 0 moved off the spiral-stair ground entry (2026-07-28): it sat at
  // (11, -9), right on stairPointAt(0) — the "cone blocking the stairs".
  { x: 5.5, z: 8 },
  { x: -ROTUNDA_RADIUS + 6, z: -ROTUNDA_RADIUS + 5 },
  { x: -ROTUNDA_RADIUS + 5, z: ROTUNDA_RADIUS - 6 },
  { x: ROTUNDA_RADIUS - 6, z: ROTUNDA_RADIUS - 7 },
];

const poolGeo = new THREE.CircleGeometry(1.1, 24);
const poolMat = new THREE.MeshStandardMaterial({
  color: WATER.surface,
  roughness: 0.12,
  metalness: 0.5,
  transparent: true,
  opacity: 0.8,
});
// The grove tree-cones were removed 2026-07-28 (ruling: low-poly green cones
// read as noise, not growth). The POOLS stay — quiet reflecting circles that
// still reveal with Brain maturity — and the night fireflies still gather
// over them (NightAmbience reads GROVE_SLOTS).
function CrownGroves() {
  const signals = getWorldSignals();
  const fullness = fullnessFromMemories(signals.memoryCount);
  // Reveal pools progressively with the Brain's maturity. 0 ⇒ none.
  const reveal = Math.round(fullness * GROVE_SLOTS.length);
  const slots = GROVE_SLOTS.slice(0, reveal);

  const poolTransforms = useMemo<InstanceTransform[]>(
    () => slots.map((s) => ({ position: [s.x, 0.05, s.z], rotation: [-Math.PI / 2, 0, 0] })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [reveal]
  );

  if (slots.length === 0) return null;
  return (
    <InstancedProp name="water.pool" geometry={poolGeo} material={poolMat} transforms={poolTransforms} receiveShadow />
  );
}

/**
 * Full water + growth system. Spring + river bound to real memory presence/flow;
 * crown pools + groves revealed by Brain maturity. Mounted from Atmosphere.tsx.
 * Re-reads the live signal store every frame (spring/river) and every poll
 * (groves) — but renders fully dormant when the Brain is empty (honesty law).
 */
export function Water() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  // Re-render CrownGroves when the slow signal store changes (poll cadence).
  // (Spring/River read the store directly in useFrame — no React churn there.)
  return (
    <>
      <Spring reduceMotion={reduceMotion} />
      <River reduceMotion={reduceMotion} />
      <CrownGrovesReactive />
    </>
  );
}

// Thin wrapper so the grove reveal re-renders on real signal changes only.
function CrownGrovesReactive() {
  useWorldSignals(); // subscribe → re-render on real poll/event updates
  return <CrownGroves />;
}
