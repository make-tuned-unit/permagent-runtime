// Main hall structure — platform, rotunda floor + circuits, columns, dome,
// station pedestals (threshold markers), orbital arcs, light shaft.
// Moved verbatim from WorldScene.tsx in the bible §5 skeleton split (W1).

import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import { Float } from '@react-three/drei';
import * as THREE from 'three';
import { COLORS, STATIONS, COLUMN_COUNT, ROTUNDA_RADIUS, DOME_HEIGHT, PLATFORM_RADIUS } from '../../constants';
import { isPunchedAngle } from '../zones';

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
  const ringRefs = useRef<(THREE.Mesh | null)[]>([]);
  const lineRefs = useRef<(THREE.Mesh | null)[]>([]);

  const radii = useMemo(() => [3, 6, 9, 12], []);
  const maxRadius = 12;

  // Distance-based falloff: inner rings bright, outer rings dimmer
  useFrame(() => {
    ringRefs.current.forEach((mesh, i) => {
      if (!mesh) return;
      const mat = mesh.material as THREE.MeshBasicMaterial;
      const distanceFade = 1 - (radii[i] / maxRadius) * 0.6; // outer rings fade to 40% of inner
      mat.opacity = distanceFade * (0.35 + 0.2 * Math.sin(performance.now() * 0.001 + i * 0.5));
    });
    lineRefs.current.forEach((mesh, i) => {
      if (!mesh) return;
      const mat = mesh.material as THREE.MeshBasicMaterial;
      // Radial lines: subtle pulse, moderate falloff
      mat.opacity = 0.2 + 0.1 * Math.sin(performance.now() * 0.0008 + i * 0.7);
    });
  });

  const rings = useMemo(() => {
    return radii.map((r) => {
      const geo = new THREE.TorusGeometry(r, 0.03, 4, 64);
      return { geo, r };
    });
  }, [radii]);

  const lineAngles = useMemo(() => {
    const count = 8;
    return Array.from({ length: count }, (_, i) => (i / count) * Math.PI * 2);
  }, []);

  // Use thin 3D torus rings + box beams raised well above floor to avoid z-fighting
  return (
    <group position-y={0.12}>
      {rings.map(({ geo }, i) => (
        <mesh key={`ring-${i}`} ref={(el) => { ringRefs.current[i] = el; }} rotation-x={-Math.PI / 2} geometry={geo}>
          <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.4} depthWrite={false} />
        </mesh>
      ))}
      {lineAngles.map((angle, i) => (
        <mesh
          key={`line-${i}`}
          ref={(el) => { lineRefs.current[i] = el; }}
          position={[Math.cos(angle) * 6, 0, Math.sin(angle) * 6]}
          rotation={[0, -angle + Math.PI / 2, 0]}
        >
          <boxGeometry args={[12, 0.06, 0.04]} />
          <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.25} depthWrite={false} />
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

  // Threshold columns are punched out of the colonnade (bible §3); the portal
  // frames in areas/Thresholds.tsx provide flanking columns in their place.
  const columns = useMemo(() => {
    return Array.from({ length: COLUMN_COUNT }, (_, i) => {
      const angle = (i / COLUMN_COUNT) * Math.PI * 2;
      const x = Math.cos(angle) * (ROTUNDA_RADIUS - 1);
      const z = Math.sin(angle) * (ROTUNDA_RADIUS - 1);
      return { x, z, angle };
    }).filter(({ angle }) => !isPunchedAngle(angle));
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
            <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.7} />
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
          {/* Rim light (point light at column edge) — subtle cyan wash */}
          <pointLight position={[0.7, DOME_HEIGHT * 0.7, 0]} color={COLORS.neonCyan} intensity={0.15} distance={4} decay={2} />
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

// Large sweeping orbital arcs — the signature dynamic element
// OrbitalArcs moved to atmosphere/Atmosphere.tsx (W4 reactive version)

// Light shaft from oculus (fake volumetric)
// LightShaft moved to atmosphere/Atmosphere.tsx (W4 reactive version)

// Hall structure composition — geometry only; lighting/fog live in atmosphere/.
export function HallStructure({
  onHoverStation,
  onClickStation,
}: {
  onHoverStation: (id: string | null) => void;
  onClickStation: (id: string) => void;
}) {
  return (
    <>
      <Platform />
      <RotundaFloor />
      <Columns />
      <Dome />
      <StationPedestals onHoverStation={onHoverStation} onClickStation={onClickStation} />

      {/* Orbital arcs — signature dynamic visual */}

      {/* Light shaft from oculus */}
    </>
  );
}
