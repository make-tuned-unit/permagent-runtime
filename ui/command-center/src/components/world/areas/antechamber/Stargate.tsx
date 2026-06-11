import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
// Stargate — large ring structure with segmented chevrons and animated event horizon

const RING_RADIUS = 3.2;
const RING_TUBE = 0.25;
const CHEVRON_COUNT = 9;
const GATE_COLOR = '#4A5468'; // gunmetal ring
const CHEVRON_COLOR = '#A08555'; // bronze chevrons
const HORIZON_COLOR = '#5599FF'; // blue-white energy

// Event horizon shader — animated ripple/vortex effect
const HORIZON_VERT = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}`;

const HORIZON_FRAG = `
uniform float uTime;
uniform vec3 uColor;
varying vec2 vUv;

void main() {
  vec2 c = vUv - 0.5;
  float r = length(c);
  float angle = atan(c.y, c.x);

  // Radial ripples expanding from center
  float ripple1 = sin(r * 18.0 - uTime * 2.5) * 0.5 + 0.5;
  float ripple2 = sin(r * 12.0 + uTime * 1.8 + angle * 3.0) * 0.5 + 0.5;

  // Spiral pattern
  float spiral = sin(angle * 4.0 + r * 8.0 - uTime * 1.2) * 0.5 + 0.5;

  // Combine
  float pattern = ripple1 * 0.4 + ripple2 * 0.3 + spiral * 0.3;

  // Fade at edges (soft circular mask)
  float edgeFade = smoothstep(0.5, 0.35, r);
  // Bright core
  float coreBright = smoothstep(0.3, 0.0, r) * 0.3;

  float alpha = (pattern * 0.6 + 0.2 + coreBright) * edgeFade;
  vec3 col = uColor * (0.8 + pattern * 0.4) + vec3(coreBright * 0.5);

  gl_FragColor = vec4(col, alpha);
}`;

function EventHorizon() {
  const matRef = useRef<THREE.ShaderMaterial>(null);

  useFrame(() => {
    if (matRef.current) {
      matRef.current.uniforms.uTime.value = performance.now() * 0.001;
    }
  });

  const uniforms = useMemo(() => ({
    uTime: { value: 0 },
    uColor: { value: new THREE.Color(HORIZON_COLOR) },
  }), []);

  return (
    <mesh>
      <circleGeometry args={[RING_RADIUS - RING_TUBE - 0.1, 64]} />
      <shaderMaterial
        ref={matRef}
        vertexShader={HORIZON_VERT}
        fragmentShader={HORIZON_FRAG}
        uniforms={uniforms}
        transparent
        side={THREE.DoubleSide}
        depthWrite={false}
        blending={THREE.AdditiveBlending}
      />
    </mesh>
  );
}

function Chevron({ angle }: { angle: number }) {
  const x = Math.cos(angle) * RING_RADIUS;
  const y = Math.sin(angle) * RING_RADIUS;

  return (
    <group position={[x, y, 0]} rotation-z={angle - Math.PI / 2}>
      {/* Chevron body — wedge shape */}
      <mesh>
        <boxGeometry args={[0.35, 0.5, 0.35]} />
        <meshStandardMaterial
          color={CHEVRON_COLOR}
          roughness={0.3}
          metalness={0.7}
        />
      </mesh>
      {/* Chevron emissive indicator — lights up */}
      <mesh position-y={0.05}>
        <boxGeometry args={[0.2, 0.12, 0.36]} />
        <meshStandardMaterial
          color={HORIZON_COLOR}
          emissive={HORIZON_COLOR}
          emissiveIntensity={1.5}
          roughness={0.1}
          metalness={0.3}
        />
      </mesh>
    </group>
  );
}

function GateRing() {
  // Main torus ring
  const ringMat = useMemo(() => new THREE.MeshStandardMaterial({
    color: GATE_COLOR,
    roughness: 0.25,
    metalness: 0.7,
  }), []);

  // Inner track — slightly smaller torus with emissive trim
  const trackMat = useMemo(() => new THREE.MeshStandardMaterial({
    color: HORIZON_COLOR,
    emissive: HORIZON_COLOR,
    emissiveIntensity: 0.6,
    roughness: 0.2,
    metalness: 0.4,
    transparent: true,
    opacity: 0.8,
  }), []);

  return (
    <group>
      {/* Main ring body */}
      <mesh material={ringMat}>
        <torusGeometry args={[RING_RADIUS, RING_TUBE, 16, 64]} />
      </mesh>

      {/* Inner emissive track */}
      <mesh material={trackMat}>
        <torusGeometry args={[RING_RADIUS - RING_TUBE * 0.5, 0.06, 8, 64]} />
      </mesh>

      {/* Outer emissive track */}
      <mesh material={trackMat}>
        <torusGeometry args={[RING_RADIUS + RING_TUBE * 0.5, 0.04, 8, 64]} />
      </mesh>

      {/* Segmented panel lines on the ring — 32 segments */}
      {Array.from({ length: 32 }, (_, i) => {
        const a = (i / 32) * Math.PI * 2;
        const x = Math.cos(a) * RING_RADIUS;
        const y = Math.sin(a) * RING_RADIUS;
        return (
          <mesh key={`seg-${i}`} position={[x, y, 0]} rotation-z={a}>
            <boxGeometry args={[0.02, RING_TUBE * 2.2, RING_TUBE * 2.2]} />
            <meshStandardMaterial
              color={GATE_COLOR}
              roughness={0.3}
              metalness={0.8}
            />
          </mesh>
        );
      })}

      {/* Chevrons */}
      {Array.from({ length: CHEVRON_COUNT }, (_, i) => {
        const angle = (i / CHEVRON_COUNT) * Math.PI * 2 + Math.PI / 2; // top-center start
        return <Chevron key={`chev-${i}`} angle={angle} />;
      })}
    </group>
  );
}

function GateParticles() {
  const ref = useRef<THREE.Points>(null);

  const { positions, velocities } = useMemo(() => {
    const count = 60;
    const pos = new Float32Array(count * 3);
    const vel = new Float32Array(count * 3);
    for (let i = 0; i < count; i++) {
      const angle = Math.random() * Math.PI * 2;
      const r = Math.random() * (RING_RADIUS - RING_TUBE - 0.2);
      pos[i * 3] = Math.cos(angle) * r;
      pos[i * 3 + 1] = Math.sin(angle) * r;
      pos[i * 3 + 2] = 0;
      // Drift outward from the surface
      vel[i * 3] = (Math.random() - 0.5) * 0.3;
      vel[i * 3 + 1] = (Math.random() - 0.5) * 0.3;
      vel[i * 3 + 2] = (Math.random() * 0.5 + 0.2) * (Math.random() > 0.5 ? 1 : -1);
    }
    return { positions: pos, velocities: vel };
  }, []);

  useFrame((_, delta) => {
    if (!ref.current) return;
    const posAttr = ref.current.geometry.getAttribute('position');
    for (let i = 0; i < posAttr.count; i++) {
      let x = posAttr.getX(i) + velocities[i * 3] * delta;
      let y = posAttr.getY(i) + velocities[i * 3 + 1] * delta;
      let z = posAttr.getZ(i) + velocities[i * 3 + 2] * delta;
      // Reset particles that drift too far
      if (Math.abs(z) > 2 || Math.sqrt(x * x + y * y) > RING_RADIUS) {
        const angle = Math.random() * Math.PI * 2;
        const r = Math.random() * (RING_RADIUS - RING_TUBE - 0.3);
        x = Math.cos(angle) * r;
        y = Math.sin(angle) * r;
        z = 0;
      }
      posAttr.setXYZ(i, x, y, z);
    }
    posAttr.needsUpdate = true;
  });

  return (
    <points ref={ref}>
      <bufferGeometry>
        <bufferAttribute attach="attributes-position" count={positions.length / 3} array={positions} itemSize={3} />
      </bufferGeometry>
      <pointsMaterial
        color={HORIZON_COLOR}
        size={0.08}
        sizeAttenuation
        transparent
        opacity={0.7}
        blending={THREE.AdditiveBlending}
        depthWrite={false}
      />
    </points>
  );
}

// Electrical arcs — stochastic lightning bolts jumping along the ring
const ARC_COUNT = 5;
const ARC_SEGMENTS = 5; // zigzag segments per arc
const ARC_COLOR = '#AACCFF';

interface ArcState {
  startAngle: number;
  endAngle: number;
  birthTime: number;
  lifetime: number; // 0.3-0.5s
  groundArc: boolean; // occasionally arcs to the pedestal
  jitter: Float32Array; // perpendicular offsets for zigzag
}

function ElectricalArcs() {
  const groupRef = useRef<THREE.Group>(null);
  const arcsRef = useRef<ArcState[]>([]);
  const linesRef = useRef<THREE.Line[]>([]);
  const initialized = useRef(false);

  // Create Line objects imperatively (R3F doesn't have a <line> element)
  useFrame(() => {
    if (!groupRef.current) return;
    const now = performance.now() * 0.001;

    // Initialize on first frame
    if (!initialized.current) {
      for (let i = 0; i < ARC_COUNT; i++) {
        arcsRef.current.push(spawnArc(now - i * 0.15));
        const geo = new THREE.BufferGeometry();
        geo.setAttribute('position', new THREE.Float32BufferAttribute(new Float32Array((ARC_SEGMENTS + 1) * 3), 3));
        const mat = new THREE.LineBasicMaterial({
          color: ARC_COLOR,
          transparent: true,
          opacity: 1.0,
          blending: THREE.AdditiveBlending,
          depthWrite: false,
        });
        const line = new THREE.Line(geo, mat);
        linesRef.current.push(line);
        groupRef.current.add(line);
      }
      initialized.current = true;
    }

    for (let i = 0; i < ARC_COUNT; i++) {
      const arc = arcsRef.current[i];
      const line = linesRef.current[i];
      if (!arc || !line) continue;
      const age = now - arc.birthTime;

      // Respawn expired arcs
      if (age > arc.lifetime) {
        arcsRef.current[i] = spawnArc(now);
        updateArcGeometry(line, arcsRef.current[i], 1.0);
        continue;
      }

      // Flicker: rapid opacity oscillation
      const t = age / arc.lifetime;
      const fadeIn = Math.min(t * 10, 1);
      const fadeOut = 1 - t * t;
      const flicker = 0.6 + 0.4 * Math.sin(now * 40 + i * 7);
      const opacity = fadeIn * fadeOut * flicker;

      updateArcGeometry(line, arc, opacity);
    }
  });

  return <group ref={groupRef} />;
}

function updateArcGeometry(line: THREE.Line, arc: ArcState, opacity: number) {
  const positions = line.geometry.getAttribute('position') as THREE.BufferAttribute;

  for (let s = 0; s <= ARC_SEGMENTS; s++) {
    const t = s / ARC_SEGMENTS;

    if (arc.groundArc) {
      const angle = arc.startAngle;
      const ringX = Math.cos(angle) * RING_RADIUS;
      const ringY = Math.sin(angle) * RING_RADIUS;
      const groundY = -(RING_RADIUS + 0.1);
      const x = ringX * (1 - t) + arc.jitter[s] * 0.3;
      const y = ringY * (1 - t) + groundY * t + arc.jitter[s] * 0.15;
      const z = arc.jitter[s] * 0.2;
      positions.setXYZ(s, x, y, z);
    } else {
      const angle = arc.startAngle + (arc.endAngle - arc.startAngle) * t;
      const baseX = Math.cos(angle) * RING_RADIUS;
      const baseY = Math.sin(angle) * RING_RADIUS;
      const perpX = Math.cos(angle) * arc.jitter[s] * 0.25;
      const perpY = Math.sin(angle) * arc.jitter[s] * 0.25;
      const z = arc.jitter[s] * 0.15;
      positions.setXYZ(s, baseX + perpX, baseY + perpY, z);
    }
  }
  positions.needsUpdate = true;
  (line.material as THREE.LineBasicMaterial).opacity = opacity;
}

function spawnArc(birthTime: number): ArcState {
  const startAngle = Math.random() * Math.PI * 2;
  const groundArc = Math.random() < 0.2; // 1 in 5 arcs ground to pedestal
  const span = 0.3 + Math.random() * 0.5; // arc covers 0.3-0.8 radians of ring
  const jitter = new Float32Array(ARC_SEGMENTS + 1);
  for (let i = 0; i < jitter.length; i++) {
    jitter[i] = (Math.random() - 0.5) * 2; // -1 to 1
  }
  // Endpoints are on the ring, so no jitter
  jitter[0] = 0;
  jitter[ARC_SEGMENTS] = 0;

  return {
    startAngle,
    endAngle: startAngle + span,
    birthTime,
    lifetime: 0.3 + Math.random() * 0.2,
    groundArc,
    jitter,
  };
}

export function StargatePortal({ position = [0, 0, 0] as [number, number, number] }) {
  return (
    <group position={position}>
      {/* Ring stands upright — rotate to face forward */}
      <group position-y={RING_RADIUS + 0.3}>
        <GateRing />
        <EventHorizon />
        <GateParticles />
        <ElectricalArcs />

        {/* Scene lighting from the event horizon */}
        <pointLight color={HORIZON_COLOR} intensity={1.2} distance={8} decay={2} />
        <pointLight color={HORIZON_COLOR} intensity={0.4} distance={12} decay={2} position-z={2} />
      </group>

      {/* Base pedestal — the gate sits on a raised platform */}
      <mesh position-y={0.15}>
        <cylinderGeometry args={[1.8, 2.2, 0.3, 16]} />
        <meshStandardMaterial color={GATE_COLOR} roughness={0.3} metalness={0.6} />
      </mesh>
      {/* Pedestal trim ring */}
      <mesh position-y={0.31}>
        <torusGeometry args={[1.8, 0.04, 6, 32]} />
        <meshStandardMaterial
          color={HORIZON_COLOR}
          emissive={HORIZON_COLOR}
          emissiveIntensity={0.8}
          roughness={0.2}
        />
      </mesh>
      {/* Ramp / steps leading up */}
      <mesh position={[0, 0.08, 2]} rotation-x={-0.08}>
        <boxGeometry args={[1.6, 0.06, 1.5]} />
        <meshStandardMaterial color={GATE_COLOR} roughness={0.3} metalness={0.5} />
      </mesh>
    </group>
  );
}
