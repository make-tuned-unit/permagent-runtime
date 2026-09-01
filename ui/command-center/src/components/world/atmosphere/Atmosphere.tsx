// W4 atmosphere — lighting, fog, starfield, dust, light shaft, orbital arcs.
// Extracted from WorldScene.tsx per WORLD_VIEW_BIBLE.md §5 (lane owns the
// move). Adds bible §7 activity-reactive ambience (dust rate, arc speed) and
// §8 reduceMotion static fallbacks. LAW: zero per-frame allocations.

import { useMemo, useRef, useEffect } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { DOME_HEIGHT } from '../constants';
import { getReduceMotion } from '../../../styles/tokens';
import { tickAmbience, getAmbienceLevel, setAmbienceFrozen } from './ambience';
import { startWorldSignals } from './worldSignals';
import { getTimeOfDay, startTimeOfDay } from './timeOfDay';
import { MouthShaft } from './MouthShaft';
import { Water } from './Water';
import { NightAmbience } from './NightAmbience';
import { FrozenShadows } from './FrozenShadows';

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
  // The warm bounce that used to come off the mezzanine shelves, and the cold
  // stone it lands on. Same temperatures as the two hall pointLights this
  // replaced (#ffc48a / #ffb87a warm, against the platform's dark stone).
  bounceSky: '#FFC48A',
  bounceGround: '#1C2033',
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

/**
 * Starts the READ-ONLY Brain/event signal store (atmosphere/worldSignals) on
 * mount — the honesty source for water presence + the day-one shadows. No daemon
 * code, no new endpoints: it polls /api/brain/graph and listens to /events,
 * which the frontend already consumes. Disposes on unmount.
 */
function SignalBinder() {
  useEffect(() => startWorldSignals(), []);
  return null;
}

/** Starts the real-clock time-of-day sampler (30s cadence, never per frame). */
function TimeBinder() {
  useEffect(() => startTimeOfDay(), []);
  return null;
}

/**
 * DEV-ONLY: stash the live three.js scene on window so the CDP evidence harness
 * can run the light census (count PointLight/DirectionalLight, shadow casters) on
 * the real scene graph. No-op in production builds.
 */
function SceneCensusHook() {
  const scene = useThree((s) => s.scene);
  useEffect(() => {
    if (!import.meta.env.DEV || typeof window === 'undefined') return;
    (window as unknown as { __worldScene?: THREE.Scene }).__worldScene = scene;
    return () => {
      delete (window as unknown as { __worldScene?: THREE.Scene }).__worldScene;
    };
  }, [scene]);
  return null;
}

// Bible §1: one warm key + one cool fill + near-black ambient. Exactly one
// shadow caster scene-wide; shadow map 2048 (drops to 1024 only if the budget
// demands). Zone re-aims land as a follow-up once W1's blockouts merge.
//
// TIME OF DAY (atmosphere/timeOfDay — the REAL local clock): the key's color
// temperature and all three intensities damp toward the current hour's
// keyframes. Midday IS the §1 baseline; night cools and dims the same lights
// rather than adding any — light count and the single shadow caster are
// untouched. The per-frame work is three scalar damps + one color damp (zero
// allocation); it stays active under reduceMotion because an hours-scale
// lighting drift is not motion.
const _keyTarget = new THREE.Color(LIGHT.key);
let _lastKeyHex = '';

function Lighting() {
  const keyRef = useRef<THREE.DirectionalLight>(null);
  const fillRef = useRef<THREE.DirectionalLight>(null);
  const ambientRef = useRef<THREE.AmbientLight>(null);
  const bounceRef = useRef<THREE.HemisphereLight>(null);

  useFrame((_, dt) => {
    const tod = getTimeOfDay();
    const key = keyRef.current;
    const fill = fillRef.current;
    const ambient = ambientRef.current;
    if (!key || !fill || !ambient) return;
    if (tod.keyColor !== _lastKeyHex) {
      _lastKeyHex = tod.keyColor;
      _keyTarget.set(tod.keyColor);
    }
    // Two-sided clamp (defense in depth, WORLD_VIEW bugfix): dt is normally
    // >= 0, but if a caller ever hands this a negative delta (e.g. a driven
    // clock that briefly regresses), a bare `Math.min(1, dt * 0.8)` would let
    // a negative factor through and drive `lerp` backward past its target.
    const k = THREE.MathUtils.clamp(dt * 0.8, 0, 1);
    key.color.lerp(_keyTarget, k);
    key.intensity = THREE.MathUtils.damp(key.intensity, tod.keyIntensity, 0.8, dt);
    fill.intensity = THREE.MathUtils.damp(fill.intensity, tod.fillIntensity, 0.8, dt);
    ambient.intensity = THREE.MathUtils.damp(ambient.intensity, tod.ambientIntensity, 0.8, dt);
    const bounce = bounceRef.current;
    if (bounce) {
      // Rides the fill's curve — it is the same kind of light, arriving from
      // above instead of from the side. Kept deliberately low: the two point
      // lights it replaced had decay 2, so their warmth died within a few
      // metres of the rotunda floor, while a hemisphere light reaches every
      // upward-facing surface in the world with no falloff at all. Matching
      // their nominal intensity would quietly raise the whole scene's exposure
      // and cost the deep-space mood the bible's near-black ambient is there
      // to protect (§1).
      bounce.intensity = THREE.MathUtils.damp(bounce.intensity, tod.fillIntensity * 0.55, 0.8, dt);
    }
  });

  return (
    <>
      <ambientLight ref={ambientRef} intensity={0.08} color={LIGHT.ambient} />
      {/* The warm bounce from above. This is a substitution, not an addition:
          it took over from the two 20-unit warm pointLights that sat on the
          rotunda floor in areas/hall/HallStructure. A hemisphere light is the
          right shape for "light bouncing down off the shelves" — it grades
          from a warm sky to a cold ground with no position and no falloff — and
          a forward renderer evaluates it far more cheaply than a point light,
          which every lit fragment on screen has to loop over whether or not it
          is anywhere near it. Damped with the rest of the light rig below, so
          the bounce cools and dims at night with everything else. */}
      <hemisphereLight
        ref={bounceRef}
        intensity={0.14}
        color={LIGHT.bounceSky}
        groundColor={LIGHT.bounceGround}
      />
      {/* Warm key light from above-and-to-the-side — the single shadow caster */}
      <directionalLight
        ref={keyRef}
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
      <directionalLight ref={fillRef} position={[-10, DOME_HEIGHT, -6]} intensity={0.25} color={LIGHT.fill} />
      {/* NOTE: the old oculus uplight pointLight was removed here. The world's
          light now comes from the Mesh portal far above — MouthShaft's single
          cold key light replaces this warm uplight, so the atmosphere lane's
          point-light census stays NET NEUTRAL (was 1 oculus → now 1 portal key). */}
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
    if (ref.current) {
      // Stars pale toward midday, deepen at night (real clock — not motion,
      // so this runs under reduceMotion too).
      const mat = ref.current.material as THREE.PointsMaterial;
      mat.opacity = THREE.MathUtils.damp(mat.opacity, 0.65 * getTimeOfDay().starOpacity, 0.8, delta);
      if (!reduceMotion) ref.current.rotation.y += delta * 0.01;
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

// ---------------------------------------------------------------------------
// Antechamber fog bias (bible §3: A5 has the densest fog, +0.004).
//
// FogExp2 is scene-global, so a literal per-zone density is impossible; W1
// shipped a haze-shell mesh as the *exterior* read (the chamber looks denser
// from the hall). This component is the *interior* read: when the camera
// crosses the Antechamber approach (NW diagonal, §3 coordinates — fixed even
// though W1's zone geometry is still on PR #294), the scene fog density ramps
// smoothly from 0.012 toward 0.016 and back out again. The two mechanisms are
// complementary, not redundant — the haze shell stays.
//
// LAW: zero per-frame allocations — the existing scene fog object is mutated
// in place. The ramp is positional (it follows camera movement, no autonomous
// animation), so it needs no reduceMotion fallback; damping only smooths the
// step when the camera teleports (tour start, mode switches).

const FOG_BASE_DENSITY = 0.012;
const FOG_ANTE_BIAS = 0.004;
const ANTE_X = -19;
const ANTE_Z = -19;
const ANTE_RADIUS_FULL = 5; // fully biased inside this ground radius
const ANTE_RADIUS_EDGE = 11; // bias begins here (just outside the gate)

// DEV-ONLY A/B toggle for evidence capture (mirrors __worldPost's pattern).
let fogBiasEnabled = true;
declare global {
  interface Window {
    __worldFog?: { setAntechamberBias: (on: boolean) => void };
  }
}
if (import.meta.env.DEV && typeof window !== 'undefined') {
  window.__worldFog = {
    setAntechamberBias: (on: boolean) => {
      fogBiasEnabled = on;
    },
  };
}

function AntechamberFogBias() {
  useFrame(({ camera, scene }, delta) => {
    const fog = scene.fog;
    if (!(fog instanceof THREE.FogExp2)) return;
    let target = FOG_BASE_DENSITY;
    if (fogBiasEnabled) {
      const dx = camera.position.x - ANTE_X;
      const dz = camera.position.z - ANTE_Z;
      const d = Math.sqrt(dx * dx + dz * dz);
      const t = THREE.MathUtils.clamp(
        (ANTE_RADIUS_EDGE - d) / (ANTE_RADIUS_EDGE - ANTE_RADIUS_FULL),
        0,
        1
      );
      target = FOG_BASE_DENSITY + FOG_ANTE_BIAS * t;
    }
    fog.density = THREE.MathUtils.damp(fog.density, target, 4, delta);
  });

  return null;
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
      <SignalBinder />
      <TimeBinder />
      {import.meta.env.DEV && <SceneCensusHook />}
      <Lighting />
      <FrozenShadows />
      <Starfield reduceMotion={reduceMotion} />
      <OrbitalArcs reduceMotion={reduceMotion} />
      <LightShaft reduceMotion={reduceMotion} />
      <DustMotes reduceMotion={reduceMotion} />
      {/* Night layers — nebula band + grove fireflies (real clock × real
          Brain-maturity gates; see NightAmbience.tsx). */}
      <NightAmbience />

      {/* Atmosphere detail pass — all bind to REAL signals or render dormant;
          none add a shadow caster; the Mesh portal adds exactly one cold point
          light. */}
      {/* The Mesh portal's daylight shaft — the light of the world. */}
      <MouthShaft />
      {/* Water + growth — the user; presence bound to real memory count. */}
      <Water />

      {/* Depth atmosphere — exponential fog for natural falloff (bible §1:
          density 0.012 base; the Antechamber biases it +0.004 on approach,
          see AntechamberFogBias above). */}
      <fogExp2 attach="fog" args={[ENV.deepVoid, 0.012]} />
      <AntechamberFogBias />
    </>
  );
}

// Distant void grid below the platform — the "distant void" floor beneath the rotunda.
export function DistantGrid() {
  return (
    <mesh rotation-x={-Math.PI / 2} position-y={-30}>
      <planeGeometry args={[400, 400, 80, 80]} />
      <meshBasicMaterial color={ENV.floorGridGlow} wireframe transparent opacity={0.15} />
    </mesh>
  );
}
