import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { COLORS } from './constants';
import { StargatePortal } from './Stargate';

// Shared materials
function useMarbleMat() {
  return useMemo(
    () => new THREE.MeshStandardMaterial({ color: COLORS.primaryMarble, roughness: 0.35, metalness: 0.05 }),
    []
  );
}

function useDarkStoneMat() {
  return useMemo(
    () => new THREE.MeshStandardMaterial({ color: '#2A2A3E', roughness: 0.3, metalness: 0.15 }),
    []
  );
}

function useWoodMat() {
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

function ReadingDesk({ position }: { position: [number, number, number] }) {
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

// Library is now on the mezzanine — see MezzanineLibrary below

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

function ObservatoryArea() {
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
      {/* Stargate portal */}
      <StargatePortal position={[0, 0, -3]} />
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

function ArmChair({ position, rotation = 0 }: { position: [number, number, number]; rotation?: number }) {
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

// === MEZZANINE LIBRARY (The Brain) ===
// Raised ring walkway high on the columns with built-in bookshelf walls

export const MEZZ_HEIGHT = 10;       // raised high so ground floor breathes
export const MEZZ_INNER_R = 12.5;
export const MEZZ_OUTER_R = 15.5;
const MEZZ_MID_R = (MEZZ_INNER_R + MEZZ_OUTER_R) / 2;

const STAIR_GAP_CENTER = Math.PI * 0.375; // between columns 1 and 2 (67.5 deg)
const STAIR_GAP_HALF = 0.12;           // small opening, ~3.4 units of arc at r=14
const SHELF_WALL_HEIGHT = 4;

function isInStairGap(angle: number): boolean {
  let diff = angle - STAIR_GAP_CENTER;
  while (diff > Math.PI) diff -= Math.PI * 2;
  while (diff < -Math.PI) diff += Math.PI * 2;
  return Math.abs(diff) < STAIR_GAP_HALF;
}

// RingGeometry lives in XY, rotated -PI/2 to XZ. This negates the Z mapping.
// To place a gap at world angle A, use ring angle = (2PI - A).
function ringAngle(worldA: number): number {
  return ((2 * Math.PI - worldA) % (2 * Math.PI) + 2 * Math.PI) % (2 * Math.PI);
}

function MezzanineRing() {
  const darkStone = useDarkStoneMat();
  const rGapCenter = ringAngle(STAIR_GAP_CENTER);
  const rStart = rGapCenter + STAIR_GAP_HALF;
  const rLength = Math.PI * 2 - STAIR_GAP_HALF * 2;

  return (
    <group position-y={MEZZ_HEIGHT}>
      {/* Inner half of ring floor — CONTINUOUS, no gap (bookshelf wall sits on this) */}
      <mesh rotation-x={-Math.PI / 2} receiveShadow>
        <ringGeometry args={[MEZZ_INNER_R, MEZZ_MID_R, 64, 1, 0, Math.PI * 2]} />
        <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.3} metalness={0.1} />
      </mesh>
      {/* Outer half of ring floor — has small stair gap */}
      <mesh rotation-x={-Math.PI / 2} receiveShadow>
        <ringGeometry args={[MEZZ_MID_R, MEZZ_OUTER_R, 64, 1, rStart, rLength]} />
        <meshStandardMaterial color={COLORS.primaryMarble} roughness={0.3} metalness={0.1} />
      </mesh>

      {/* Outer railing — skip stair gap */}
      {Array.from({ length: 48 }, (_, i) => {
        const angle = (i / 48) * Math.PI * 2;
        if (isInStairGap(angle)) return null;
        return (
          <mesh key={`op-${i}`} position={[Math.cos(angle) * MEZZ_OUTER_R, 0.4, Math.sin(angle) * MEZZ_OUTER_R]} material={darkStone}>
            <cylinderGeometry args={[0.04, 0.04, 0.8, 4]} />
          </mesh>
        );
      })}
      <mesh position-y={0.65} rotation-x={-Math.PI / 2}>
        <ringGeometry args={[MEZZ_OUTER_R - 0.03, MEZZ_OUTER_R + 0.03, 64, 1, rStart, rLength]} />
        <primitive object={darkStone} attach="material" />
      </mesh>

      {/* Cyan glow on outer edge */}
      <mesh position-y={0.02}>
        <torusGeometry args={[MEZZ_OUTER_R - 0.05, 0.03, 4, 64]} />
        <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.3} depthWrite={false} />
      </mesh>
    </group>
  );
}

function Staircase() {
  const marble = useMarbleMat();
  const stepCount = 30;  // more steps for the greater height
  const endAngle = STAIR_GAP_CENTER;
  const arcSpan = Math.PI * 0.6; // wider spiral arc
  const startAngle = endAngle - arcSpan;
  const stairR = MEZZ_MID_R;
  const stepWidth = MEZZ_OUTER_R - MEZZ_INNER_R - 0.4;

  return (
    <group>
      {Array.from({ length: stepCount }, (_, i) => {
        const t = i / (stepCount - 1);
        const angle = startAngle + t * arcSpan;
        // Last step is exactly at MEZZ_HEIGHT (flush with floor)
        const y = t * MEZZ_HEIGHT;
        const x = Math.cos(angle) * stairR;
        const z = Math.sin(angle) * stairR;
        return (
          <mesh key={i} position={[x, y + 0.05, z]} rotation-y={-angle + Math.PI / 2} material={marble}>
            <boxGeometry args={[stepWidth, 0.15, 0.6]} />
          </mesh>
        );
      })}
      {/* Railings on both sides */}
      {Array.from({ length: stepCount }, (_, i) => {
        if (i % 3 !== 0) return null;
        const t = i / (stepCount - 1);
        const angle = startAngle + t * arcSpan;
        const y = t * MEZZ_HEIGHT;
        return (
          <group key={`r-${i}`}>
            <mesh position={[Math.cos(angle) * (MEZZ_INNER_R + 0.15), y + 0.5, Math.sin(angle) * (MEZZ_INNER_R + 0.15)]}>
              <cylinderGeometry args={[0.04, 0.04, 1, 4]} />
              <meshStandardMaterial color="#888" metalness={0.5} roughness={0.3} />
            </mesh>
            <mesh position={[Math.cos(angle) * (MEZZ_OUTER_R - 0.15), y + 0.5, Math.sin(angle) * (MEZZ_OUTER_R - 0.15)]}>
              <cylinderGeometry args={[0.04, 0.04, 1, 4]} />
              <meshStandardMaterial color="#888" metalness={0.5} roughness={0.3} />
            </mesh>
          </group>
        );
      })}
    </group>
  );
}

// Continuous built-in bookshelf wall on the INNER edge of the ring.
// CylinderGeometry angles match world-space. RingGeometry needs conversion.
function BookshelfWall() {
  const wood = useWoodMat();
  const darkStone = useDarkStoneMat();

  // Cylinder: world-space angles directly
  const cylStart = STAIR_GAP_CENTER + STAIR_GAP_HALF;
  const cylLength = Math.PI * 2 - STAIR_GAP_HALF * 2;
  // Ring geometry: needs mirrored angles
  const rStart = ringAngle(STAIR_GAP_CENTER) + STAIR_GAP_HALF;
  const rLength = Math.PI * 2 - STAIR_GAP_HALF * 2;

  const wallR = MEZZ_INNER_R + 0.05;
  const shelfR = MEZZ_INNER_R + 0.35;
  const shelfCount = 5;
  const shelfDepth = 0.3;

  // Books use world-space angles (positioned with cos/sin)
  const bookRows = useMemo(() => {
    const rows: { angle: number; shelfY: number; width: number; height: number; hue: number }[] = [];
    const booksPerShelf = 80;
    for (let s = 0; s < shelfCount; s++) {
      const shelfY = 0.15 + s * (SHELF_WALL_HEIGHT / shelfCount);
      for (let b = 0; b < booksPerShelf; b++) {
        // Distribute books around the ring in world-space, skipping stair gap
        const angle = ((b / booksPerShelf) * Math.PI * 2);
        if (isInStairGap(angle)) continue;
        rows.push({
          angle,
          shelfY,
          width: 0.06 + Math.random() * 0.04,
          height: 0.4 + Math.random() * 0.3,
          hue: (s * 50 + b * 17) % 360,
        });
      }
    }
    return rows;
  }, []);

  return (
    <group position-y={MEZZ_HEIGHT + 0.01}>
      {/* Back wall — cylinder uses world-space angles */}
      <mesh position-y={SHELF_WALL_HEIGHT / 2} material={darkStone}>
        <cylinderGeometry args={[wallR, wallR, SHELF_WALL_HEIGHT, 64, 1, true,
          cylStart, cylLength]} />
      </mesh>

      {/* Horizontal shelf planks — ring geometry needs converted angles */}
      {Array.from({ length: shelfCount + 1 }, (_, i) => {
        const y = i * (SHELF_WALL_HEIGHT / shelfCount);
        return (
          <mesh key={`shelf-${i}`} position-y={y} rotation-x={-Math.PI / 2}>
            <ringGeometry args={[wallR, shelfR, 64, 1, rStart, rLength]} />
            <primitive object={wood} attach="material" />
          </mesh>
        );
      })}

      {/* Vertical dividers — world-space positions */}
      {Array.from({ length: 48 }, (_, i) => {
        const angle = (i / 48) * Math.PI * 2;
        if (isInStairGap(angle)) return null;
        const midR = (wallR + shelfR) / 2;
        const x = Math.cos(angle) * midR;
        const z = Math.sin(angle) * midR;
        return (
          <mesh key={`div-${i}`} position={[x, SHELF_WALL_HEIGHT / 2, z]} rotation-y={-angle}>
            <boxGeometry args={[0.04, SHELF_WALL_HEIGHT, shelfDepth]} />
            <primitive object={wood} attach="material" />
          </mesh>
        );
      })}

      {/* Books — world-space positions */}
      {bookRows.map((book, i) => {
        const bookR = wallR + shelfDepth * 0.4;
        const x = Math.cos(book.angle) * bookR;
        const z = Math.sin(book.angle) * bookR;
        return (
          <mesh key={`book-${i}`} position={[x, book.shelfY + book.height / 2, z]} rotation-y={-book.angle}>
            <boxGeometry args={[book.width, book.height, 0.18]} />
            <meshStandardMaterial color={`hsl(${book.hue}, 35%, 30%)`} roughness={0.8} />
          </mesh>
        );
      })}
    </group>
  );
}

function MezzanineLibraryContents() {
  return (
    <group position-y={MEZZ_HEIGHT + 0.15}>
      {/* Reading desks along inner walkway */}
      {Array.from({ length: 6 }, (_, i) => {
        const angle = (i / 6) * Math.PI * 2 + Math.PI / 6;
        if (isInStairGap(angle)) return null;
        const r = MEZZ_INNER_R + 1.2;
        return <ReadingDesk key={`desk-${i}`} position={[Math.cos(angle) * r, 0, Math.sin(angle) * r]} />;
      })}
      {/* Armchairs */}
      <ArmChair position={[Math.cos(Math.PI) * MEZZ_MID_R, 0, Math.sin(Math.PI) * MEZZ_MID_R]} rotation={0} />
      <ArmChair position={[Math.cos(0) * MEZZ_MID_R, 0, Math.sin(0) * MEZZ_MID_R]} rotation={Math.PI} />
      {/* Warm amber lighting */}
      {Array.from({ length: 8 }, (_, i) => {
        const angle = (i / 8) * Math.PI * 2;
        return (
          <pointLight key={i} position={[Math.cos(angle) * MEZZ_MID_R, 2.5, Math.sin(angle) * MEZZ_MID_R]} color={COLORS.neonAmber} intensity={0.2} distance={8} />
        );
      })}
    </group>
  );
}

// === MAIN EXPORT ===

export function LabFurniture() {
  return (
    <group>
      <WorkbenchArea />
      <ObservatoryArea />
      <ForumArea />
      {/* Mezzanine Library (The Brain) */}
      <MezzanineRing />
      <Staircase />
      <BookshelfWall />
      <MezzanineLibraryContents />
    </group>
  );
}
