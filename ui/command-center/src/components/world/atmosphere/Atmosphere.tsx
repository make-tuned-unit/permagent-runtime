// W4 atmosphere — lighting, fog, starfield, dust, light shaft, orbital arcs.
// Extracted from WorldScene.tsx per WORLD_VIEW_BIBLE.md §5 (lane owns the
// move). Adds bible §7 activity-reactive ambience (dust rate, arc speed) and
// §8 reduceMotion static fallbacks. LAW: zero per-frame allocations.

import { useMemo, useRef, useEffect } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { DOME_HEIGHT } from '../constants';
import { getReduceMotion } from '../../../styles/tokens';
import { tickAmbience, getAmbienceLevel, setAmbienceFrozen } from './ambience';

// Bible §1 light formula: one warm key (shadows), one cool fill (no shadows),
// near-black ambient 0.08. These are the established §1 colors — they are
// light temperatures, not palette accents, so they live here rather than in
// the frozen shared/palette.ts.
const LIGHT = {
  ambient: '#B8C4D8',
  key: '#FFF0D4',
  fill: '#8EC8E8',
  oculus: '#FFF8E7',
  shaft: '#FFFAE5',
} as const;

/**
 * Advances the shared ambience level exactly once per frame, before every
 * other useFrame consumer (priority -100), so all reactive elements read a
 * consistent value within a frame.
 */
function AmbienceTicker({ reduceMotion }: { reduceMotion: boolean }) {
  useEffect(() => {
    setAmbienceFrozen(reduceMotion);
  }, [reduceMotion]);

  useFrame((_, delta) => {
    tickAmbience(delta);
  }, -100);

  return null;
}

// Bible §1: one warm key + one cool fill + near-black ambient. Exactly one
// shadow caster scene-wide; shadow map 2048 (drops to 1024 only if the budget
// demands). Zone re-aims land as a follow-up once W1's blockouts merge.
function Lighting() {
  return (
    <>
      <ambientLight intensity={0.08} color={LIGHT.ambient} />
      {/* Warm key light from above-and-to-the-side — the single shadow caster */}
      <directionalLight
        position={[12, DOME_HEIGHT + 8, 8]}
        intensity={1.6}
        color={LIGHT.key}
        castShadow
        shadow-mapSize-width={2048}
        shadow-mapSize-height={2048}
        shadow-bias={-0.0005}
        shadow-camera-near={18}
        shadow-camera-far={38}
        shadow-camera-left={-20}
        shadow-camera-right={20}
        shadow-camera-top={20}
        shadow-camera-bottom={-20}
      />
      {/* Cool fill light from opposite side — prevents pure black shadows */}
      <directionalLight position={[-10, DOME_HEIGHT, -6]} intensity={0.25} color={LIGHT.fill} />
      {/* Uplight from oculus shaft — faint warm wash on dome interior */}
      <pointLight
        position={[0, 2, 0]}
        color={LIGHT.oculus}
        intensity={0.4}
        distance={DOME_HEIGHT + 5}
        decay={2}
      />
    </>
  );
}

// Starfield background. reduceMotion: static (no rotation).
function Starfield({ reduceMotion }: { reduceMotion: boolean }) {
  const ref = useRef<THREE.Points>(null);

  const positions = useMemo(() => {
    const count = 2000;
    const pos = new Float32Array(count * 3);
    for (let i = 0; i < count; i++) {
      const r = 80 + Math.random() * 200;
      const theta = Math.random() * Math.PI * 2;
      const phi = Math.random() * Math.PI;
      pos[i * 3] = r * Math.sin(phi) * Math.cos(theta);
      pos[i * 3 + 1] = r * Math.cos(phi) - 40;
      pos[i * 3 + 2] = r * Math.sin(phi) * Math.sin(theta);
    }
    return pos;
  }, []);

  useFrame((_, delta) => {
    if (reduceMotion) return;
    if (ref.current) {
      ref.current.rotation.y += delta * 0.01;
    }
  });

  return (
    <points ref={ref}>
      <bufferGeometry>
        <bufferAttribute attach="attributes-position" count={positions.length / 3} array={positions} itemSize={3} />
      </bufferGeometry>
      <pointsMaterial color="#FFFFFF" size={0.3} sizeAttenuation transparent opacity={0.6} />
    </points>
  );
}

// Dust motes in the light shaft. Reactive ambience: rise rate and density
// read scale with the working count (≤1.5×). reduceMotion: static motes.
function DustMotes({ reduceMotion }: { reduceMotion: boolean }) {
  const ref = useRef<THREE.Points>(null);

  const positions = useMemo(() => {
    const count = 200;
    const pos = new Float32Array(count * 3);
    for (let i = 0; i < count; i++) {
      const r = Math.random() * 3;
      const theta = Math.random() * Math.PI * 2;
      pos[i * 3] = Math.cos(theta) * r;
      pos[i * 3 + 1] = Math.random() * DOME_HEIGHT;
      pos[i * 3 + 2] = Math.sin(theta) * r;
    }
    return pos;
  }, []);

  useFrame((_, delta) => {
    if (reduceMotion) return;
    if (ref.current) {
      const level = getAmbienceLevel();
      const posAttr = ref.current.geometry.getAttribute('position');
      for (let i = 0; i < posAttr.count; i++) {
        let y = posAttr.getY(i) + delta * 0.3 * level;
        if (y > DOME_HEIGHT) y = 0;
        posAttr.setY(i, y);
      }
      posAttr.needsUpdate = true;
      // Busy room reads slightly denser: opacity 0.5 idle → 0.75 at full busy.
      const mat = ref.current.material as THREE.PointsMaterial;
      mat.opacity = 0.5 * level;
    }
  });

  return (
    <points ref={ref}>
      <bufferGeometry>
        <bufferAttribute attach="attributes-position" count={positions.length / 3} array={positions} itemSize={3} />
      </bufferGeometry>
      <pointsMaterial color={ENV.neonAmber} size={0.05} sizeAttenuation transparent opacity={0.5} />
    </points>
  );
}

// Light shaft from oculus (fake volumetric). reduceMotion: constant opacity.
function LightShaft({ reduceMotion }: { reduceMotion: boolean }) {
  const ref = useRef<THREE.Mesh>(null);

  useFrame(() => {
    if (reduceMotion) return;
    if (ref.current) {
      const mat = ref.current.material as THREE.MeshBasicMaterial;
      mat.opacity = (0.09 + 0.03 * Math.sin(performance.now() * 0.0005)) * getAmbienceLevel();
    }
  });

  return (
    <mesh ref={ref} position-y={DOME_HEIGHT / 2}>
      <cylinderGeometry args={[2.5, 4, DOME_HEIGHT, 32, 1, true]} />
      <meshBasicMaterial color={LIGHT.shaft} transparent opacity={0.1} side={THREE.DoubleSide} depthWrite={false} />
    </mesh>
  );
}

interface ArcSpec {
  radius: number;
  tiltX: number;
  tiltZ: number;
  arcLength: number;
}

const ARCS: ArcSpec[] = [
  { radius: 11, tiltX: 0.4, tiltZ: 0.2, arcLength: Math.PI * 1.3 },
  { radius: 13, tiltX: -0.3, tiltZ: 0.5, arcLength: Math.PI * 1.5 },
  { radius: 9, tiltX: 0.6, tiltZ: -0.3, arcLength: Math.PI * 1.1 },
];

function setArcParticle(p: THREE.Object3D, arc: ArcSpec, phase: number): void {
  p.position.set(
    Math.cos(phase) * arc.radius,
    Math.sin(phase) * arc.radius * Math.sin(arc.tiltX),
    Math.sin(phase) * arc.radius * Math.cos(arc.tiltX)
  );
}

// Large sweeping orbital arcs — the signature dynamic element. Reactive
// ambience: drift + particle speed scale with the working count (≤1.5×).
// Particle motion uses accumulated phases (not wall-clock time) so speed
// changes ramp smoothly instead of jumping. reduceMotion: static arcs,
// particles parked at their initial phase.
function OrbitalArcs({ reduceMotion }: { reduceMotion: boolean }) {
  const groupRef = useRef<THREE.Group>(null);
  const particlesRef = useRef<THREE.Group>(null);
  // Accumulated phase per particle — mutated in place, never reallocated.
  const phases = useRef<Float32Array>(
    Float32Array.from(ARCS.map((_, i) => i * 2.1))
  );

  useFrame((_, delta) => {
    if (reduceMotion) return;
    const level = getAmbienceLevel();
    if (groupRef.current) {
      groupRef.current.rotation.y += delta * 0.015 * level;
    }
    if (particlesRef.current) {
      const children = particlesRef.current.children;
      for (let i = 0; i < children.length; i++) {
        const speed = (0.3 + i * 0.1) * level;
        phases.current[i] = (phases.current[i] + delta * speed) % (Math.PI * 2);
        setArcParticle(children[i], ARCS[i % ARCS.length], phases.current[i]);
      }
    }
  });

  // Park particles at their initial phase so reduceMotion doesn't leave them
  // clumped at the group origin.
  useEffect(() => {
    const group = particlesRef.current;
    if (!group) return;
    group.children.forEach((p, i) => {
      setArcParticle(p, ARCS[i % ARCS.length], phases.current[i]);
    });
  }, []);

  // Build gradient opacity via vertex colors on each arc
  const arcMeshes = useMemo(() => {
    return ARCS.map((arc) => {
      const segments = 128;
      const geo = new THREE.TorusGeometry(arc.radius, 0.04, 6, segments, arc.arcLength);
      const colors = new Float32Array(geo.attributes.position.count * 3);
      for (let i = 0; i < geo.attributes.position.count; i++) {
        const t = i / geo.attributes.position.count;
        // Fade from bright center to dim edges
        const brightness = 0.3 + 0.7 * Math.sin(t * Math.PI);
        colors[i * 3] = brightness;
        colors[i * 3 + 1] = brightness;
        colors[i * 3 + 2] = brightness;
      }
      geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
      return { geo, ...arc };
    });
  }, []);

  return (
    <group ref={groupRef} position-y={DOME_HEIGHT * 0.5}>
      {arcMeshes.map((arc, i) => (
        <mesh key={`arc-${i}`} geometry={arc.geo} rotation-x={arc.tiltX} rotation-z={arc.tiltZ}>
          <meshBasicMaterial
            color={ENV.neonCyan}
            transparent
            opacity={0.5}
            depthWrite={false}
            vertexColors
          />
        </mesh>
      ))}
      {/* Traveling particles along arcs */}
      <group ref={particlesRef}>
        {ARCS.map((_, i) => (
          <mesh key={`particle-${i}`}>
            <sphereGeometry args={[0.1, 8, 8]} />
            <meshBasicMaterial color={ENV.neonCyan} transparent opacity={0.9} />
          </mesh>
        ))}
      </group>
    </group>
  );
}

/**
 * Full atmosphere stack for the hall: lighting, fog, starfield, dust, light
 * shaft, orbital arcs, and the reactive-ambience ticker.
 */
export function Atmosphere() {
  // Static accessibility setting — read once per mount.
  const reduceMotion = useMemo(() => getReduceMotion(), []);

  return (
    <>
      <AmbienceTicker reduceMotion={reduceMotion} />
      <Lighting />
      <Starfield reduceMotion={reduceMotion} />
      <OrbitalArcs reduceMotion={reduceMotion} />
      <LightShaft reduceMotion={reduceMotion} />
      <DustMotes reduceMotion={reduceMotion} />
      {/* Depth atmosphere — exponential fog for natural falloff (bible §1:
          density 0.012 everywhere; zones may bias ±0.004 after W1 lands). */}
      <fogExp2 attach="fog" args={[ENV.deepVoid, 0.012]} />
    </>
  );
}

// Distant void grid below the platform — kept from the Phase-A verbatim move
// (not part of the reactive stack; mounted separately by WorldScene).
export function DistantGrid() {
  return (
    <mesh rotation-x={-Math.PI / 2} position-y={-30}>
      <planeGeometry args={[400, 400, 80, 80]} />
      <meshBasicMaterial color={ENV.floorGridGlow} wireframe transparent opacity={0.15} />
    </mesh>
  );
}
