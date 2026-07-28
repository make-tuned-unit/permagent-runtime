// Legacy prop library — moved verbatim from world/WorldFurniture.tsx in the
// bible §5 skeleton split (W1). W2 takes ownership: instancing + anchor registry.
// The mezzanine library moved to areas/hall/MezzanineLibrary.tsx (part of the hall).

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { COLORS } from '../../constants';
import { makeStoneTexture } from '../../shared/stoneTexture';

// Shared materials
export function useMarbleMat() {
  return useMemo(
    // #16 realism pass: marble props carry the shared stone canvas texture
    // (speckle + veining) instead of a flat color — stairs, benches, trim.
    () => new THREE.MeshStandardMaterial({ map: makeStoneTexture('#aeb4c0'), roughness: 0.35, metalness: 0.05 }),
    []
  );
}

export function useDarkStoneMat() {
  return useMemo(
    () => new THREE.MeshStandardMaterial({ color: '#2A2A3E', roughness: 0.3, metalness: 0.15 }),
    []
  );
}

export function useWoodMat() {
  return useMemo(
    () => new THREE.MeshStandardMaterial({ color: '#5C4033', roughness: 0.6, metalness: 0 }),
    []
  );
}

function useFabricMat(color: string) {
  return useMemo(
    () => new THREE.MeshStandardMaterial({ color, roughness: 0.85, metalness: 0 }),
    [color]
  );
}

function useCyanGlow() {
  return useMemo(
    () => new THREE.MeshBasicMaterial({ color: COLORS.neonCyan, transparent: true, opacity: 0.6 }),
    []
  );
}

// === WORKBENCH AREA (North, z=-10) ===
// Large work table with holographic screens, tools, drawers

function WorkTable({ position }: { position: [number, number, number] }) {
  const wood = useWoodMat();
  const darkStone = useDarkStoneMat();

  return (
    <group position={position}>
      {/* Tabletop */}
      <mesh position-y={0.9} castShadow material={darkStone}>
        <boxGeometry args={[3.5, 0.12, 1.4]} />
      </mesh>
      {/* Legs */}
      {[[-1.5, 0, -0.5], [-1.5, 0, 0.5], [1.5, 0, -0.5], [1.5, 0, 0.5]].map((p, i) => (
        <mesh key={i} position={[p[0], 0.44, p[2]]} castShadow material={wood}>
          <boxGeometry args={[0.1, 0.88, 0.1]} />
        </mesh>
      ))}
      {/* Drawers (front) */}
      {[-0.8, 0, 0.8].map((x, i) => (
        <mesh key={`d-${i}`} position={[x, 0.6, 0.65]} material={wood}>
          <boxGeometry args={[0.6, 0.25, 0.08]} />
        </mesh>
      ))}
      {/* Drawer handles */}
      {[-0.8, 0, 0.8].map((x, i) => (
        <mesh key={`h-${i}`} position={[x, 0.6, 0.7]}>
          <boxGeometry args={[0.15, 0.03, 0.03]} />
          <meshStandardMaterial color="#888" metalness={0.8} roughness={0.2} />
        </mesh>
      ))}
      {/* Circuit inlay on tabletop */}
      <mesh position={[0, 0.97, 0]} rotation-x={-Math.PI / 2}>
        <ringGeometry args={[0.3, 0.33, 32]} />
        <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.4} depthWrite={false} />
      </mesh>
    </group>
  );
}

function HoloScreen({ position, width = 1.2, height = 0.8 }: { position: [number, number, number]; width?: number; height?: number }) {
  const ref = useRef<THREE.Mesh>(null);

  // Random line widths must be stable across re-renders — Math.random() in
  // geometry args makes R3F rebuild the BufferGeometry on every render (leak).
  const lineWidths = useMemo(
    () => Array.from({ length: 5 }, () => width * 0.5 + Math.random() * width * 0.2),
    [width]
  );

  useFrame(() => {
    if (ref.current) {
      const mat = ref.current.material as THREE.MeshBasicMaterial;
      mat.opacity = 0.15 + 0.05 * Math.sin(performance.now() * 0.002);
    }
  });

  return (
    <group position={position}>
      {/* Screen panel */}
      <mesh ref={ref}>
        <planeGeometry args={[width, height]} />
        <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.18} side={THREE.DoubleSide} depthWrite={false} />
      </mesh>
      {/* Border frame */}
      <mesh>
        <boxGeometry args={[width + 0.04, height + 0.04, 0.01]} />
        <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.3} depthWrite={false} />
      </mesh>
      {/* "Content" lines */}
      {lineWidths.map((w, i) => (
        <mesh key={i} position={[-width * 0.3, height * 0.3 - i * height * 0.15, 0.01]}>
          <boxGeometry args={[w, 0.02, 0.001]} />
          <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.25} depthWrite={false} />
        </mesh>
      ))}
    </group>
  );
}

function ToolRack({ position }: { position: [number, number, number] }) {
  const darkStone = useDarkStoneMat();
  return (
    <group position={position}>
      {/* Back panel */}
      <mesh material={darkStone}>
        <boxGeometry args={[1.5, 1.8, 0.1]} />
      </mesh>
      {/* Shelves */}
      {[0.5, 0, -0.5].map((y, i) => (
        <mesh key={i} position={[0, y, 0.1]} material={darkStone}>
          <boxGeometry args={[1.4, 0.06, 0.25]} />
        </mesh>
      ))}
      {/* Tool items (cylinders as flasks/tubes) */}
      {[-0.4, -0.1, 0.2, 0.5].map((x, i) => (
        <mesh key={`t-${i}`} position={[x, 0.6, 0.15]}>
          <cylinderGeometry args={[0.04, 0.04, 0.2 + i * 0.05, 6]} />
          <meshBasicMaterial color={i % 2 === 0 ? COLORS.neonCyan : COLORS.neonAmber} transparent opacity={0.6} />
        </mesh>
      ))}
    </group>
  );
}

function WorkbenchArea() {
  return (
    <group position={[0, 0, -10]} rotation-y={0}>
      {/* Main work table */}
      <WorkTable position={[0, 0, 0]} />
      {/* Second smaller table */}
      <WorkTable position={[4, 0, -1]} />
      {/* Holographic screens above main table */}
      <HoloScreen position={[0, 2.2, -0.5]} width={1.8} height={1} />
      <HoloScreen position={[1.5, 2, -0.3]} width={1} height={0.7} />
      {/* Tool rack */}
      <ToolRack position={[-2.5, 0.9, -1.5]} />
      {/* Stool */}
      <Stool position={[0, 0, 1.2]} />
      <Stool position={[1.5, 0, 1.2]} />
    </group>
  );
}

export function ReadingDesk({ position }: { position: [number, number, number] }) {
  const wood = useWoodMat();
  const marble = useMarbleMat();
  return (
    <group position={position}>
      {/* Desktop */}
      <mesh position-y={0.8} castShadow material={marble}>
        <boxGeometry args={[1.8, 0.1, 1]} />
      </mesh>
      {/* Legs */}
      {[[-0.7, 0, -0.35], [-0.7, 0, 0.35], [0.7, 0, -0.35], [0.7, 0, 0.35]].map((p, i) => (
        <mesh key={i} position={[p[0], 0.4, p[2]]} material={wood}>
          <cylinderGeometry args={[0.05, 0.06, 0.8, 8]} />
        </mesh>
      ))}
      {/* Open book on desk */}
      <mesh position={[0, 0.87, 0]} rotation-x={-0.1}>
        <boxGeometry args={[0.5, 0.02, 0.35]} />
        <meshStandardMaterial color="#F5F0E0" roughness={0.9} />
      </mesh>
      {/* Desk lamp */}
      <group position={[0.7, 0.85, -0.3]}>
        <mesh>
          <cylinderGeometry args={[0.08, 0.1, 0.05, 8]} />
          <meshStandardMaterial color="#888" metalness={0.6} roughness={0.3} />
        </mesh>
        <mesh position={[0, 0.3, 0]}>
          <cylinderGeometry args={[0.015, 0.015, 0.6, 6]} />
          <meshStandardMaterial color="#888" metalness={0.6} roughness={0.3} />
        </mesh>
        <mesh position={[0, 0.55, 0.05]} rotation-x={0.3}>
          <coneGeometry args={[0.12, 0.15, 8, 1, true]} />
          <meshStandardMaterial color="#888" metalness={0.4} roughness={0.4} />
        </mesh>
        <pointLight position={[0, 0.5, 0.1]} color={COLORS.neonAmber} intensity={0.5} distance={3} />
      </group>
    </group>
  );
}

// Library is now on the mezzanine — see areas/hall/MezzanineLibrary.tsx

// === OBSERVATORY AREA (South, z=10) ===
// Armillary sphere, star chart table, observation seats

function ArmillarySphere({ position }: { position: [number, number, number] }) {
  const ref = useRef<THREE.Group>(null);

  useFrame((_, delta) => {
    if (ref.current) {
      ref.current.rotation.y += delta * 0.2;
      ref.current.rotation.z += delta * 0.1;
    }
  });

  return (
    <group position={position}>
      {/* Base pedestal */}
      <mesh position-y={0.5} castShadow>
        <cylinderGeometry args={[0.3, 0.4, 1, 8]} />
        <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.3} metalness={0.1} />
      </mesh>
      {/* Rings */}
      <group ref={ref} position-y={1.5}>
        <mesh>
          <torusGeometry args={[0.8, 0.03, 8, 32]} />
          <meshStandardMaterial color="#C0A060" metalness={0.7} roughness={0.3} />
        </mesh>
        <mesh rotation-x={Math.PI / 3}>
          <torusGeometry args={[0.7, 0.03, 8, 32]} />
          <meshStandardMaterial color="#C0A060" metalness={0.7} roughness={0.3} />
        </mesh>
        <mesh rotation-x={-Math.PI / 4} rotation-y={Math.PI / 4}>
          <torusGeometry args={[0.6, 0.03, 8, 32]} />
          <meshStandardMaterial color="#C0A060" metalness={0.7} roughness={0.3} />
        </mesh>
        {/* Central orb */}
        <mesh>
          <sphereGeometry args={[0.15, 16, 16]} />
          <meshBasicMaterial color={COLORS.neonAmber} transparent opacity={0.7} />
        </mesh>
        <pointLight color={COLORS.neonAmber} intensity={0.4} distance={3} />
      </group>
    </group>
  );
}

function StarChartTable({ position }: { position: [number, number, number] }) {
  const marble = useMarbleMat();
  const ref = useRef<THREE.Group>(null);

  useFrame(() => {
    if (ref.current) {
      ref.current.rotation.y += 0.001;
    }
  });

  return (
    <group position={position}>
      {/* Circular table */}
      <mesh position-y={0.75} castShadow material={marble}>
        <cylinderGeometry args={[1.2, 1.2, 0.08, 32]} />
      </mesh>
      {/* Central pillar */}
      <mesh position-y={0.375} material={marble}>
        <cylinderGeometry args={[0.2, 0.3, 0.75, 8]} />
      </mesh>
      {/* Star chart hologram on table */}
      <group ref={ref} position-y={0.85}>
        {/* Constellation dots */}
        {Array.from({ length: 20 }, (_, i) => {
          const a = (i / 20) * Math.PI * 2;
          const r = 0.3 + Math.random() * 0.7;
          return (
            <mesh key={i} position={[Math.cos(a) * r, Math.random() * 0.3, Math.sin(a) * r]}>
              <sphereGeometry args={[0.02, 4, 4]} />
              <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.7} />
            </mesh>
          );
        })}
      </group>
    </group>
  );
}

// Unrendered since the zone blockouts: the Lab (areas/lab) absorbed the
// observatory corner (bible §3 A3). Kept exported for W2 to mine for parts.
export function ObservatoryArea() {
  return (
    <group position={[0, 0, 10]} rotation-y={Math.PI}>
      {/* Armillary sphere centerpiece */}
      <ArmillarySphere position={[0, 0, 0]} />
      {/* Star chart table */}
      <StarChartTable position={[3, 0, 0.5]} />
      {/* Observation benches */}
      <Bench position={[-2.5, 0, 2]} rotation={-0.3} />
      <Bench position={[2.5, 0, 2]} rotation={0.3} />
      {/* Telescope */}
      <group position={[-3, 0, -1]} rotation-y={0.5}>
        {/* Tripod */}
        {[0, 2.1, 4.2].map((a, i) => (
          <mesh key={i} position={[Math.cos(a) * 0.3, 0.6, Math.sin(a) * 0.3]} rotation-z={Math.cos(a) * 0.2} rotation-x={Math.sin(a) * 0.2}>
            <cylinderGeometry args={[0.03, 0.03, 1.3, 6]} />
            <meshStandardMaterial color="#888" metalness={0.6} roughness={0.3} />
          </mesh>
        ))}
        {/* Tube */}
        <mesh position={[0, 1.2, 0]} rotation-x={-0.4}>
          <cylinderGeometry args={[0.08, 0.12, 1.2, 8]} />
          <meshStandardMaterial color="#C0A060" metalness={0.5} roughness={0.3} />
        </mesh>
      </group>
    </group>
  );
}

// === FORUM PORTAL AREA (West, x=-10) ===
// Lounge with couches, low table, the portal gateway

// Legacy portal — kept for reference, replaced by StargatePortal
export function PortalGateway({ position }: { position: [number, number, number] }) {
  const ref = useRef<THREE.Mesh>(null);

  useFrame(() => {
    if (ref.current) {
      ref.current.rotation.z += 0.005;
      const mat = ref.current.material as THREE.MeshBasicMaterial;
      mat.opacity = 0.15 + 0.1 * Math.sin(performance.now() * 0.001);
    }
  });

  return (
    <group position={position}>
      {/* Archway frame */}
      <mesh position-y={2}>
        <torusGeometry args={[1.5, 0.15, 8, 32, Math.PI]} />
        <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.3} metalness={0.15} />
      </mesh>
      {/* Left pillar */}
      <mesh position={[-1.5, 1, 0]}>
        <cylinderGeometry args={[0.15, 0.18, 2, 8]} />
        <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.3} />
      </mesh>
      {/* Right pillar */}
      <mesh position={[1.5, 1, 0]}>
        <cylinderGeometry args={[0.15, 0.18, 2, 8]} />
        <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.3} />
      </mesh>
      {/* Swirling vortex inside */}
      <mesh ref={ref} position-y={1.8}>
        <torusGeometry args={[1, 0.3, 8, 32]} />
        <meshBasicMaterial color={COLORS.neonAmber} transparent opacity={0.2} depthWrite={false} />
      </mesh>
      {/* Inner glow */}
      <pointLight position={[0, 1.8, 0]} color={COLORS.neonAmber} intensity={0.6} distance={4} />
    </group>
  );
}

function ForumArea() {
  return (
    <group position={[-10, 0, 0]} rotation-y={Math.PI / 2}>
      {/* The Stargate relocated to the NW Antechamber threshold (areas/Thresholds) */}
      {/* Semicircle of couches facing portal */}
      <Couch position={[-3, 0, 3]} rotation={-0.4} />
      <Couch position={[0, 0, 4]} rotation={0} />
      <Couch position={[3, 0, 3]} rotation={0.4} />
      {/* Low central table */}
      <LowTable position={[0, 0, 3]} />
    </group>
  );
}

// === SHARED FURNITURE PIECES ===

function Couch({ position, rotation = 0 }: { position: [number, number, number]; rotation?: number }) {
  const fabric = useFabricMat('#2A2A4E');
  const trim = useCyanGlow();
  return (
    <group position={position} rotation-y={rotation}>
      {/* Seat */}
      <mesh position-y={0.35} castShadow material={fabric}>
        <boxGeometry args={[1.8, 0.3, 0.7]} />
      </mesh>
      {/* Backrest */}
      <mesh position={[0, 0.65, -0.3]} castShadow material={fabric}>
        <boxGeometry args={[1.8, 0.5, 0.15]} />
      </mesh>
      {/* Armrests */}
      <mesh position={[-0.85, 0.5, 0]} material={fabric}>
        <boxGeometry args={[0.12, 0.35, 0.7]} />
      </mesh>
      <mesh position={[0.85, 0.5, 0]} material={fabric}>
        <boxGeometry args={[0.12, 0.35, 0.7]} />
      </mesh>
      {/* Legs */}
      {[[-0.75, -0.25], [-0.75, 0.25], [0.75, -0.25], [0.75, 0.25]].map(([x, z], i) => (
        <mesh key={i} position={[x, 0.1, z]}>
          <cylinderGeometry args={[0.03, 0.03, 0.2, 6]} />
          <meshStandardMaterial color="#888" metalness={0.5} roughness={0.3} />
        </mesh>
      ))}
      {/* Cyan trim along base */}
      <mesh position={[0, 0.21, 0.35]} material={trim}>
        <boxGeometry args={[1.75, 0.02, 0.02]} />
      </mesh>
    </group>
  );
}

export function ArmChair({ position, rotation = 0 }: { position: [number, number, number]; rotation?: number }) {
  const fabric = useFabricMat('#3A2A2E');
  return (
    <group position={position} rotation-y={rotation}>
      {/* Seat */}
      <mesh position-y={0.35} castShadow material={fabric}>
        <boxGeometry args={[0.8, 0.25, 0.7]} />
      </mesh>
      {/* Backrest */}
      <mesh position={[0, 0.65, -0.3]} castShadow material={fabric}>
        <boxGeometry args={[0.8, 0.5, 0.15]} />
      </mesh>
      {/* Armrests */}
      <mesh position={[-0.38, 0.5, 0]} material={fabric}>
        <boxGeometry args={[0.1, 0.25, 0.6]} />
      </mesh>
      <mesh position={[0.38, 0.5, 0]} material={fabric}>
        <boxGeometry args={[0.1, 0.25, 0.6]} />
      </mesh>
      {/* Legs */}
      {[[-0.3, -0.25], [-0.3, 0.25], [0.3, -0.25], [0.3, 0.25]].map(([x, z], i) => (
        <mesh key={i} position={[x, 0.1, z]}>
          <cylinderGeometry args={[0.03, 0.03, 0.2, 6]} />
          <meshStandardMaterial color="#5C4033" roughness={0.6} />
        </mesh>
      ))}
    </group>
  );
}

function Bench({ position, rotation = 0 }: { position: [number, number, number]; rotation?: number }) {
  const marble = useMarbleMat();
  return (
    <group position={position} rotation-y={rotation}>
      {/* Seat slab */}
      <mesh position-y={0.45} castShadow material={marble}>
        <boxGeometry args={[1.5, 0.12, 0.5]} />
      </mesh>
      {/* Supports */}
      <mesh position={[-0.5, 0.2, 0]} material={marble}>
        <boxGeometry args={[0.12, 0.4, 0.45]} />
      </mesh>
      <mesh position={[0.5, 0.2, 0]} material={marble}>
        <boxGeometry args={[0.12, 0.4, 0.45]} />
      </mesh>
    </group>
  );
}

function Stool({ position }: { position: [number, number, number] }) {
  return (
    <group position={position}>
      {/* Seat */}
      <mesh position-y={0.65} castShadow>
        <cylinderGeometry args={[0.2, 0.2, 0.06, 12]} />
        <meshStandardMaterial color="#2A2A3E" roughness={0.5} metalness={0.1} />
      </mesh>
      {/* Stem */}
      <mesh position-y={0.35}>
        <cylinderGeometry args={[0.04, 0.04, 0.6, 6]} />
        <meshStandardMaterial color="#888" metalness={0.6} roughness={0.3} />
      </mesh>
      {/* Base */}
      <mesh position-y={0.05}>
        <cylinderGeometry args={[0.2, 0.22, 0.1, 12]} />
        <meshStandardMaterial color="#888" metalness={0.6} roughness={0.3} />
      </mesh>
    </group>
  );
}

function LowTable({ position }: { position: [number, number, number] }) {
  const marble = useMarbleMat();
  return (
    <group position={position}>
      {/* Top */}
      <mesh position-y={0.4} castShadow material={marble}>
        <cylinderGeometry args={[0.8, 0.8, 0.08, 16]} />
      </mesh>
      {/* Central pillar */}
      <mesh position-y={0.2} material={marble}>
        <cylinderGeometry args={[0.12, 0.15, 0.4, 8]} />
      </mesh>
      {/* Base */}
      <mesh position-y={0.03} material={marble}>
        <cylinderGeometry args={[0.4, 0.45, 0.06, 12]} />
      </mesh>
    </group>
  );
}

// === MAIN EXPORT ===

export function LegacyFurniture() {
  return (
    <group>
      <WorkbenchArea />
      <ForumArea />
    </group>
  );
}
