// NightAmbience — the void deepens after dark.
//
// Two decorative layers, both honest about what drives them:
//
//   • VOID NEBULA — a single far additive band that gives the starfield depth.
//     Its presence follows the REAL local clock (timeOfDay.nebulaOpacity):
//     barely-there at midday, a slow aurora at night. Claims no agent state.
//
//   • GROVE FIREFLIES — motes drifting over the crown groves. Double-gated on
//     truth: they exist only where groves exist (real Brain maturity —
//     worldSignals memory count, the same gate Water uses) AND only in the
//     night hours (real clock). An empty Brain or high noon ⇒ nothing.
//
// BUDGET: 1 mesh + 1 points = 2 draw calls, no lights. LAW: zero per-frame
// allocations (preallocated buffers, damped scalars). reduceMotion ⇒ the
// nebula holds still and the fireflies hang as fixed faint points.

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { getReduceMotion } from '../../../styles/tokens';
import { getTimeOfDay } from './timeOfDay';
import { getWorldSignals } from './worldSignals';
import { GROVE_SLOTS, fullnessFromMemories } from './Water';

// ── Void nebula band ─────────────────────────────────────────────────────────

const NEBULA_INNER = 120;
const NEBULA_OUTER = 210;

function buildNebulaGeometry(): THREE.BufferGeometry {
  // A flat ring far out in the void, vertex-colored: violet→horizon-blue drift
  // with soft alpha edges baked into the color (additive blending reads black
  // as transparent, so color IS opacity).
  const geo = new THREE.RingGeometry(NEBULA_INNER, NEBULA_OUTER, 96, 4);
  const pos = geo.attributes.position;
  const colors = new Float32Array(pos.count * 3);
  const violet = new THREE.Color(ENV.violet);
  const blue = new THREE.Color(ENV.horizonBlue);
  const c = new THREE.Color();
  for (let i = 0; i < pos.count; i++) {
    const x = pos.getX(i);
    const y = pos.getY(i);
    const r = Math.sqrt(x * x + y * y);
    const band = (r - NEBULA_INNER) / (NEBULA_OUTER - NEBULA_INNER); // 0..1
    const edge = Math.sin(band * Math.PI); // soft in/out across the band
    const swirl = 0.5 + 0.5 * Math.sin(Math.atan2(y, x) * 3);
    c.copy(violet).lerp(blue, swirl).multiplyScalar(edge * (0.35 + 0.3 * swirl));
    colors[i * 3] = c.r;
    colors[i * 3 + 1] = c.g;
    colors[i * 3 + 2] = c.b;
  }
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  return geo;
}

function VoidNebula({ reduceMotion }: { reduceMotion: boolean }) {
  const ref = useRef<THREE.Mesh>(null);
  const geo = useMemo(buildNebulaGeometry, []);
  const mat = useMemo(
    () =>
      new THREE.MeshBasicMaterial({
        vertexColors: true,
        transparent: true,
        opacity: 0.0,
        depthWrite: false,
        side: THREE.DoubleSide,
        blending: THREE.AdditiveBlending,
        toneMapped: false,
      }),
    [],
  );

  useFrame((_, dt) => {
    const m = ref.current;
    if (!m) return;
    const target = getTimeOfDay().nebulaOpacity * 0.16;
    mat.opacity = THREE.MathUtils.damp(mat.opacity, target, 1.2, dt);
    if (!reduceMotion) m.rotation.z += dt * 0.004;
  });

  return (
    <mesh
      ref={ref}
      geometry={geo}
      material={mat}
      rotation-x={-Math.PI / 2 + 0.16}
      position-y={-14}
    />
  );
}

// ── Grove fireflies ──────────────────────────────────────────────────────────

const FLY_PER_GROVE = 9;
const FLY_CAP = GROVE_SLOTS.length * FLY_PER_GROVE;

function GroveFireflies({ reduceMotion }: { reduceMotion: boolean }) {
  const ref = useRef<THREE.Points>(null);

  const { geo, seeds } = useMemo(() => {
    const g = new THREE.BufferGeometry();
    const p = new Float32Array(FLY_CAP * 3);
    const s = new Float32Array(FLY_CAP * 3); // per-fly phase seeds
    for (let i = 0; i < FLY_CAP; i++) {
      const grove = GROVE_SLOTS[Math.floor(i / FLY_PER_GROVE)];
      p[i * 3] = grove.x;
      p[i * 3 + 1] = 1.4;
      p[i * 3 + 2] = grove.z;
      s[i * 3] = Math.sin(i * 12.9898) * 43758.5453 % 1;
      s[i * 3 + 1] = Math.sin(i * 78.233) * 12543.2341 % 1;
      s[i * 3 + 2] = Math.sin(i * 39.425) * 26431.8765 % 1;
    }
    g.setAttribute('position', new THREE.BufferAttribute(p, 3));
    return { geo: g, seeds: s };
  }, []);

  const mat = useMemo(
    () =>
      new THREE.PointsMaterial({
        color: ENV.neonAmber,
        size: 0.055,
        sizeAttenuation: true,
        transparent: true,
        opacity: 0,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      }),
    [],
  );

  useFrame(({ clock }, dt) => {
    const pts = ref.current;
    if (!pts) return;
    const signals = getWorldSignals();
    // Truth gates: real groves (Brain maturity) × real night hours.
    const groveCount = Math.round(
      fullnessFromMemories(signals.memoryCount) * GROVE_SLOTS.length,
    );
    const gate = getTimeOfDay().fireflies;
    const visible = groveCount > 0 && gate > 0.02;
    const targetOpacity = visible ? 0.75 * gate : 0;
    mat.opacity = THREE.MathUtils.damp(mat.opacity, targetOpacity, 1.5, dt);
    pts.visible = mat.opacity > 0.01;
    if (!pts.visible) return;

    const drawCount = groveCount * FLY_PER_GROVE;
    geo.setDrawRange(0, drawCount);

    if (reduceMotion) return; // fixed faint points
    const t = clock.elapsedTime;
    const posAttr = geo.getAttribute('position') as THREE.BufferAttribute;
    const arr = posAttr.array as Float32Array;
    for (let i = 0; i < drawCount; i++) {
      const grove = GROVE_SLOTS[Math.floor(i / FLY_PER_GROVE)];
      const sx = seeds[i * 3];
      const sy = seeds[i * 3 + 1];
      const sz = seeds[i * 3 + 2];
      arr[i * 3] = grove.x + Math.sin(t * (0.3 + sx * 0.4) + sx * 20) * 1.1;
      arr[i * 3 + 1] = 0.7 + 0.9 * (0.5 + 0.5 * Math.sin(t * (0.5 + sy * 0.5) + sy * 30));
      arr[i * 3 + 2] = grove.z + Math.cos(t * (0.25 + sz * 0.45) + sz * 25) * 1.1;
    }
    posAttr.needsUpdate = true;
  });

  return <points ref={ref} geometry={geo} material={mat} visible={false} />;
}

export function NightAmbience() {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  return (
    <>
      <VoidNebula reduceMotion={reduceMotion} />
      <GroveFireflies reduceMotion={reduceMotion} />
    </>
  );
}
