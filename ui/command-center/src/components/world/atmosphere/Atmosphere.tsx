// Atmosphere — lighting, fog, starfield, distant grid, dust motes.
// Moved verbatim from WorldScene.tsx in the bible §5 skeleton split (W1).
// W4 takes ownership after the split lands.

import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { COLORS, DOME_HEIGHT } from '../constants';

// Starfield background
export function Starfield() {
  const ref = useRef<THREE.Points>(null);

  const [positions] = useMemo(() => {
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
    return [pos];
  }, []);

  useFrame((_, delta) => {
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

// Distant TRON-style grid plane
export function DistantGrid() {
  return (
    <mesh rotation-x={-Math.PI / 2} position-y={-30}>
      <planeGeometry args={[400, 400, 80, 80]} />
      <meshBasicMaterial color={COLORS.floorGridGlow} wireframe transparent opacity={0.15} />
    </mesh>
  );
}

// Dust motes in the light shaft
export function DustMotes() {
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
    if (ref.current) {
      const posAttr = ref.current.geometry.getAttribute('position');
      for (let i = 0; i < posAttr.count; i++) {
        let y = posAttr.getY(i) + delta * 0.3;
        if (y > DOME_HEIGHT) y = 0;
        posAttr.setY(i, y);
      }
      posAttr.needsUpdate = true;
    }
  });

  return (
    <points ref={ref}>
      <bufferGeometry>
        <bufferAttribute attach="attributes-position" count={positions.length / 3} array={positions} itemSize={3} />
      </bufferGeometry>
      <pointsMaterial color={COLORS.neonAmber} size={0.05} sizeAttenuation transparent opacity={0.5} />
    </points>
  );
}

// Scene lighting — one warm key (shadow caster), one cool fill, near-black ambient.
export function HallLighting() {
  return (
    <>
      {/* Lighting — low ambient lets neon accents read as emissive */}
      <ambientLight intensity={0.08} color="#B8C4D8" />
      {/* Warm key light from above-and-to-the-side with soft shadows */}
      <directionalLight
        position={[12, DOME_HEIGHT + 8, 8]}
        intensity={1.6}
        color="#FFF0D4"
        castShadow
        shadow-mapSize-width={2048}
        shadow-mapSize-height={2048}
        shadow-radius={4}
        shadow-bias={-0.0005}
        shadow-camera-near={18}
        shadow-camera-far={38}
        shadow-camera-left={-20}
        shadow-camera-right={20}
        shadow-camera-top={20}
        shadow-camera-bottom={-20}
      />
      {/* Cool fill light from opposite side — prevents pure black shadows */}
      <directionalLight
        position={[-10, DOME_HEIGHT, -6]}
        intensity={0.25}
        color="#8EC8E8"
      />
      {/* Uplight from oculus shaft — faint warm wash on dome interior */}
      <pointLight position={[0, 2, 0]} color="#FFF8E7" intensity={0.4} distance={DOME_HEIGHT + 5} decay={2} />
    </>
  );
}

// Depth atmosphere — exponential fog for natural falloff
export function WorldFog() {
  return <fogExp2 attach="fog" args={[COLORS.deepVoid, 0.012]} />;
}
