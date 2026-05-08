import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import { Float } from '@react-three/drei';
import * as THREE from 'three';
import { COLORS, STATIONS, COLUMN_COUNT, ROTUNDA_RADIUS, DOME_HEIGHT, PLATFORM_RADIUS } from './constants';
import { LabFurniture } from './WorldFurniture';

// Floor with glowing circuit mandala pattern
function RotundaFloor() {
  const floorMaterial = useMemo(
    () =>
      new THREE.MeshStandardMaterial({
        color: COLORS.primaryMarble,
        roughness: 0.3,
        metalness: 0.1,
        polygonOffset: true,
        polygonOffsetFactor: 1,
        polygonOffsetUnits: 1,
      }),
    []
  );

  return (
    <group position-y={0.05}>
      {/* Main marble floor — raised above platform to avoid z-fighting */}
      <mesh rotation-x={-Math.PI / 2} receiveShadow>
        <circleGeometry args={[ROTUNDA_RADIUS, 64]} />
        <primitive object={floorMaterial} attach="material" />
      </mesh>
      {/* Circuit mandala glow lines */}
      <FloorCircuits />
    </group>
  );
}

function FloorCircuits() {
  const ref = useRef<THREE.Group>(null);

  useFrame(() => {
    if (ref.current) {
      ref.current.children.forEach((child, i) => {
        if (child instanceof THREE.Mesh) {
          const mat = child.material as THREE.MeshBasicMaterial;
          mat.opacity = 0.3 + 0.2 * Math.sin(performance.now() * 0.001 + i * 0.5);
        }
      });
    }
  });

  const rings = useMemo(() => {
    const radii = [3, 6, 9, 12];
    return radii.map((r) => {
      const geo = new THREE.TorusGeometry(r, 0.03, 4, 64);
      return { geo, r };
    });
  }, []);

  const lineAngles = useMemo(() => {
    const count = 8;
    return Array.from({ length: count }, (_, i) => (i / count) * Math.PI * 2);
  }, []);

  // Use thin 3D torus rings + box beams raised well above floor to avoid z-fighting
  return (
    <group ref={ref} position-y={0.12}>
      {rings.map(({ geo }, i) => (
        <mesh key={`ring-${i}`} rotation-x={-Math.PI / 2} geometry={geo}>
          <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.4} depthWrite={false} />
        </mesh>
      ))}
      {lineAngles.map((angle, i) => (
        <mesh
          key={`line-${i}`}
          position={[Math.cos(angle) * 6, 0, Math.sin(angle) * 6]}
          rotation={[0, -angle + Math.PI / 2, 0]}
        >
          <boxGeometry args={[12, 0.06, 0.04]} />
          <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.3} depthWrite={false} />
        </mesh>
      ))}
    </group>
  );
}

// Doric columns with circuit veins
function Columns() {
  const groupRef = useRef<THREE.Group>(null);

  useFrame(() => {
    if (groupRef.current) {
      groupRef.current.children.forEach((col, i) => {
        const vein = col.children[1];
        if (vein instanceof THREE.Mesh) {
          const mat = vein.material as THREE.MeshBasicMaterial;
          mat.opacity = 0.5 + 0.3 * Math.sin(performance.now() * 0.0008 + i * 0.8);
        }
      });
    }
  });

  const columns = useMemo(() => {
    return Array.from({ length: COLUMN_COUNT }, (_, i) => {
      const angle = (i / COLUMN_COUNT) * Math.PI * 2;
      const x = Math.cos(angle) * (ROTUNDA_RADIUS - 1);
      const z = Math.sin(angle) * (ROTUNDA_RADIUS - 1);
      return { x, z, angle };
    });
  }, []);

  return (
    <group ref={groupRef}>
      {columns.map(({ x, z }, i) => (
        <group key={i} position={[x, 0, z]}>
          {/* Column body */}
          <mesh castShadow position-y={DOME_HEIGHT / 2}>
            <cylinderGeometry args={[0.6, 0.7, DOME_HEIGHT, 16]} />
            <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.4} metalness={0.05} />
          </mesh>
          {/* Circuit vein */}
          <mesh position-y={DOME_HEIGHT / 2}>
            <cylinderGeometry args={[0.15, 0.15, DOME_HEIGHT - 1, 8]} />
            <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.6} />
          </mesh>
          {/* Capital (top) */}
          <mesh position-y={DOME_HEIGHT + 0.3}>
            <cylinderGeometry args={[0.9, 0.6, 0.6, 16]} />
            <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.3} />
          </mesh>
          {/* Base */}
          <mesh position-y={0.3}>
            <cylinderGeometry args={[0.7, 0.9, 0.6, 16]} />
            <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.3} />
          </mesh>
          {/* Rim light (point light at column edge) */}
          <pointLight position={[0.7, DOME_HEIGHT * 0.7, 0]} color={COLORS.neonCyan} intensity={0.3} distance={3} />
        </group>
      ))}
    </group>
  );
}

// Dome ceiling with oculus
function Dome() {
  return (
    <group>
      {/* Dome shell */}
      <mesh position-y={DOME_HEIGHT}>
        <sphereGeometry args={[ROTUNDA_RADIUS + 1, 32, 16, 0, Math.PI * 2, 0, Math.PI / 2]} />
        <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.5} side={THREE.BackSide} />
      </mesh>
      {/* Oculus ring */}
      <mesh position-y={DOME_HEIGHT + ROTUNDA_RADIUS * 0.97} rotation-x={Math.PI / 2}>
        <ringGeometry args={[2, 3, 32]} />
        <meshStandardMaterial color={COLORS.marbleVeining} roughness={0.3} metalness={0.2} />
      </mesh>
    </group>
  );
}

// Floating obsidian platform
function Platform() {
  return (
    <group position-y={-0.5}>
      <mesh receiveShadow>
        <cylinderGeometry args={[PLATFORM_RADIUS, PLATFORM_RADIUS - 1, 1, 64]} />
        <meshStandardMaterial color="#1A1A2E" roughness={0.2} metalness={0.4} />
      </mesh>
      {/* Edge glow */}
      <mesh position-y={0.1}>
        <torusGeometry args={[PLATFORM_RADIUS - 0.5, 0.08, 8, 64]} />
        <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.4} />
      </mesh>
    </group>
  );
}

// Starfield background
function Starfield() {
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
function DistantGrid() {
  return (
    <mesh rotation-x={-Math.PI / 2} position-y={-30}>
      <planeGeometry args={[400, 400, 80, 80]} />
      <meshBasicMaterial color={COLORS.floorGridGlow} wireframe transparent opacity={0.15} />
    </mesh>
  );
}

// Station pedestals with floating holographic icons
function StationPedestals({
  onHoverStation,
  onClickStation,
}: {
  onHoverStation: (id: string | null) => void;
  onClickStation: (id: string) => void;
}) {
  return (
    <group>
      {STATIONS.map((station) => (
        <StationPedestal
          key={station.id}
          station={station}
          onHover={onHoverStation}
          onClick={onClickStation}
        />
      ))}
    </group>
  );
}

function StationPedestal({
  station,
  onHover,
  onClick,
}: {
  station: (typeof STATIONS)[number];
  onHover: (id: string | null) => void;
  onClick: (id: string) => void;
}) {
  const isPortal = station.iconType === 'portal';
  const pedestalHeight = isPortal ? 2 : 1.5;

  return (
    <group
      position={station.position}
      onPointerOver={(e) => { e.stopPropagation(); onHover(station.id); }}
      onPointerOut={(e) => { e.stopPropagation(); onHover(null); }}
      onClick={(e) => { e.stopPropagation(); onClick(station.id); }}
    >
      {/* Pedestal */}
      <mesh castShadow position-y={pedestalHeight / 2}>
        <cylinderGeometry args={[0.6, 0.8, pedestalHeight, isPortal ? 8 : 6]} />
        <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.3} metalness={0.1} />
      </mesh>
      {/* Floating icon */}
      <Float speed={2} rotationIntensity={0.3} floatIntensity={0.5}>
        <group position-y={pedestalHeight + 1.2}>
          <StationIcon type={station.iconType} />
        </group>
      </Float>
      {/* Pedestal glow ring */}
      <mesh rotation-x={-Math.PI / 2} position-y={pedestalHeight + 0.01}>
        <ringGeometry args={[0.4, 0.6, 32]} />
        <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.5} />
      </mesh>
    </group>
  );
}

function StationIcon({ type }: { type: string }) {
  const color = type === 'portal' ? COLORS.neonAmber : COLORS.neonCyan;

  switch (type) {
    case 'gear':
      return (
        <mesh>
          <torusGeometry args={[0.4, 0.1, 8, 6]} />
          <meshBasicMaterial color={color} transparent opacity={0.8} />
        </mesh>
      );
    case 'scroll':
      return (
        <mesh>
          <cylinderGeometry args={[0.15, 0.15, 0.8, 8]} />
          <meshBasicMaterial color={color} transparent opacity={0.8} />
        </mesh>
      );
    case 'planets':
      return (
        <group>
          <mesh>
            <sphereGeometry args={[0.25, 16, 16]} />
            <meshBasicMaterial color={color} transparent opacity={0.8} />
          </mesh>
          <mesh position={[0.5, 0, 0]}>
            <sphereGeometry args={[0.1, 8, 8]} />
            <meshBasicMaterial color={color} transparent opacity={0.6} />
          </mesh>
        </group>
      );
    case 'portal':
      return (
        <mesh>
          <torusGeometry args={[0.4, 0.08, 16, 32]} />
          <meshBasicMaterial color={color} transparent opacity={0.8} />
        </mesh>
      );
    default:
      return null;
  }
}

// Dust motes in the light shaft
function DustMotes() {
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

// Light shaft from oculus (fake volumetric)
function LightShaft() {
  const ref = useRef<THREE.Mesh>(null);

  useFrame(() => {
    if (ref.current) {
      const mat = ref.current.material as THREE.MeshBasicMaterial;
      mat.opacity = 0.06 + 0.02 * Math.sin(performance.now() * 0.0005);
    }
  });

  return (
    <mesh ref={ref} position-y={DOME_HEIGHT / 2}>
      <cylinderGeometry args={[2.5, 4, DOME_HEIGHT, 32, 1, true]} />
      <meshBasicMaterial color="#FFFAE5" transparent opacity={0.07} side={THREE.DoubleSide} depthWrite={false} />
    </mesh>
  );
}

// Main scene composition
export function WorldSceneContent({
  onHoverStation,
  onClickStation,
}: {
  onHoverStation: (id: string | null) => void;
  onClickStation: (id: string) => void;
}) {
  return (
    <>
      {/* Lighting */}
      <ambientLight intensity={0.3} color="#E8E4DD" />
      <directionalLight
        position={[0, DOME_HEIGHT + 10, 0]}
        intensity={1.2}
        color="#FFF8E7"
        castShadow
        shadow-mapSize-width={2048}
        shadow-mapSize-height={2048}
        shadow-camera-near={0.5}
        shadow-camera-far={50}
        shadow-camera-left={-20}
        shadow-camera-right={20}
        shadow-camera-top={20}
        shadow-camera-bottom={-20}
      />

      {/* Environment */}
      <Starfield />
      <DistantGrid />
      <Platform />
      <RotundaFloor />
      <Columns />
      <Dome />
      <StationPedestals onHoverStation={onHoverStation} onClickStation={onClickStation} />
      <LabFurniture />

      {/* Atmosphere */}
      <LightShaft />
      <DustMotes />

      {/* Fog */}
      <fog attach="fog" args={[COLORS.deepVoid, 30, 150]} />
    </>
  );
}
