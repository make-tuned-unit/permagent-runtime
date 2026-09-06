// Legacy prop library — moved verbatim from world/WorldFurniture.tsx in the
// bible §5 skeleton split (W1). W2 takes ownership: instancing + anchor registry.
// The mezzanine library moved to areas/hall/MezzanineLibrary.tsx (part of the hall).

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { COLORS } from '../../constants';
import { makeStoneTexture } from '../../shared/stoneTexture';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { unitBox, unitPlane } from '../geometries';

// Shared materials
export function useMarbleMat() {
  return useMemo(
    // #16 realism pass: marble props carry the shared stone canvas texture
    // (speckle + veining) instead of a flat color — stairs, benches, trim.
    () => new THREE.MeshLambertMaterial({ map: makeStoneTexture('#aeb4c0') }),
    []
  );
}

export function useDarkStoneMat() {
  return useMemo(
    () => new THREE.MeshLambertMaterial({ color: '#2A2A3E' }),
    []
  );
}

export function useWoodMat() {
  return useMemo(
    () => new THREE.MeshLambertMaterial({ color: '#5C4033' }),
    []
  );
}

function useFabricMat(color: string) {
  return useMemo(
    () => new THREE.MeshLambertMaterial({ color }),
    [color]
  );
}

// The two mounted legacy areas are static scene furniture. These singleton
// resources keep their old materials while allowing repeated parts to share a
// draw call; animated/interactive props remain separate below.
const legacyDarkStone = new THREE.MeshLambertMaterial({ color: '#2A2A3E' });
const legacyWood = new THREE.MeshLambertMaterial({ color: '#5C4033' });
const legacyHandle = new THREE.MeshStandardMaterial({ color: '#888', metalness: 0.8, roughness: 0.2 });
const legacyHoloPanel = new THREE.MeshBasicMaterial({ color: COLORS.neonCyan, transparent: true, opacity: 0.18, side: THREE.DoubleSide, depthWrite: false });
const legacyHoloFrame = new THREE.MeshBasicMaterial({ color: COLORS.neonCyan, transparent: true, opacity: 0.3, depthWrite: false });
const legacyHoloLine = new THREE.MeshBasicMaterial({ color: COLORS.neonCyan, transparent: true, opacity: 0.25, depthWrite: false });
const legacyToolCyan = new THREE.MeshBasicMaterial({ color: COLORS.neonCyan, transparent: true, opacity: 0.6 });
const legacyToolAmber = new THREE.MeshBasicMaterial({ color: COLORS.neonAmber, transparent: true, opacity: 0.6 });
const legacyCircuit = new THREE.MeshBasicMaterial({ color: COLORS.neonCyan, transparent: true, opacity: 0.4, depthWrite: false });
const legacyStoolSeat = new THREE.MeshLambertMaterial({ color: '#2A2A3E' });
const legacyMetal = new THREE.MeshStandardMaterial({ color: '#888', metalness: 0.6, roughness: 0.3 });
const legacyCouchFabric = new THREE.MeshLambertMaterial({ color: '#2A2A4E' });
const legacyCouchTrim = new THREE.MeshBasicMaterial({ color: COLORS.neonCyan, transparent: true, opacity: 0.6 });
const legacyCouchLeg = new THREE.MeshStandardMaterial({ color: '#888', metalness: 0.5, roughness: 0.3 });
const legacyLowTableTopGeo = new THREE.CylinderGeometry(0.8, 0.8, 0.08, 16);
const legacyLowTablePillarGeo = new THREE.CylinderGeometry(0.12, 0.15, 0.4, 8);
const legacyLowTableBaseGeo = new THREE.CylinderGeometry(0.4, 0.45, 0.06, 12);
const legacyStoolSeatGeo = new THREE.CylinderGeometry(0.2, 0.2, 0.06, 12);
const legacyStoolStemGeo = new THREE.CylinderGeometry(0.04, 0.04, 0.6, 6);
const legacyStoolBaseGeo = new THREE.CylinderGeometry(0.2, 0.22, 0.1, 12);
const legacyToolGeo = new THREE.CylinderGeometry(0.5, 0.5, 1, 6);
const legacyCouchLegGeo = new THREE.CylinderGeometry(0.5, 0.5, 1, 6);
const legacyCouchTrimGeo = unitBox;
const legacyCircuitGeo = new THREE.RingGeometry(0.3, 0.33, 32);
const legacyDeskLegGeo = new THREE.CylinderGeometry(0.05, 0.06, 0.8, 8);
const legacyDeskLampBaseGeo = new THREE.CylinderGeometry(0.08, 0.1, 0.05, 8);
const legacyDeskLampStemGeo = new THREE.CylinderGeometry(0.015, 0.015, 0.6, 6);
const legacyDeskLampShadeGeo = new THREE.ConeGeometry(0.12, 0.15, 8, 1, true);
const legacyDeskBookMat = new THREE.MeshLambertMaterial({ color: '#F5F0E0' });
const legacyDeskLampMat = new THREE.MeshStandardMaterial({ color: '#888', metalness: 0.6, roughness: 0.3 });
const legacyDeskLampShadeMat = new THREE.MeshStandardMaterial({ color: '#888', metalness: 0.4, roughness: 0.4 });
const legacyArmchairFabric = new THREE.MeshLambertMaterial({ color: '#3A2A2E' });
const legacyArmchairLeg = new THREE.MeshLambertMaterial({ color: '#5C4033' });

// === WORKBENCH AREA (North, z=-10) ===
// Large work table with holographic screens, tools, drawers. All pieces are
// static; layout is explicit so batching cannot alter the old transforms.
interface WorkbenchLayout {
  tops: InstanceTransform[];
  legs: InstanceTransform[];
  drawers: InstanceTransform[];
  handles: InstanceTransform[];
  circuits: InstanceTransform[];
  panels: InstanceTransform[];
  frames: InstanceTransform[];
  lines: InstanceTransform[];
  rackBack: InstanceTransform[];
  rackShelves: InstanceTransform[];
  toolsCyan: InstanceTransform[];
  toolsAmber: InstanceTransform[];
  stoolSeats: InstanceTransform[];
  stoolStems: InstanceTransform[];
  stoolBases: InstanceTransform[];
}

export function buildWorkbenchLayout(): WorkbenchLayout {
  const layout: WorkbenchLayout = {
    tops: [], legs: [], drawers: [], handles: [], circuits: [], panels: [], frames: [], lines: [],
    rackBack: [], rackShelves: [], toolsCyan: [], toolsAmber: [], stoolSeats: [], stoolStems: [], stoolBases: [],
  };
  const tables: [number, number][] = [[0, 0], [4, -1]];
  for (const [tx, tz] of tables) {
    layout.tops.push({ position: [tx, 0.9, tz], scale: [3.5, 0.12, 1.4] });
    for (const [x, z] of [[-1.5, -0.5], [-1.5, 0.5], [1.5, -0.5], [1.5, 0.5]] as [number, number][]) {
      layout.legs.push({ position: [tx + x, 0.44, tz + z], scale: [0.1, 0.88, 0.1] });
    }
    for (const x of [-0.8, 0, 0.8]) {
      layout.drawers.push({ position: [tx + x, 0.6, tz + 0.65], scale: [0.6, 0.25, 0.08] });
      layout.handles.push({ position: [tx + x, 0.6, tz + 0.7], scale: [0.15, 0.03, 0.03] });
    }
    layout.circuits.push({ position: [tx, 0.97, tz], rotation: [-Math.PI / 2, 0, 0] });
  }

  const screens: [number, number, number, number][] = [
    [0, 2.2, -0.5, 1.8],
    [1.5, 2, -0.3, 1],
  ];
  for (const [x, y, z, width] of screens) {
    const height = width === 1.8 ? 1 : 0.7;
    layout.panels.push({ position: [x, y, z], scale: [width, height, 1] });
    layout.frames.push({ position: [x, y, z], scale: [width + 0.04, height + 0.04, 0.01] });
    // The old widths were random in [0.5w, 0.7w]; fixed values preserve the
    // same varied line read while making the static layout deterministic.
    const widths = [0.52, 0.58, 0.64, 0.55, 0.68];
    widths.forEach((factor, i) => layout.lines.push({
      position: [x - width * 0.3, y + height * 0.3 - i * height * 0.15, z + 0.01],
      scale: [width * factor, 0.02, 0.001],
    }));
  }

  const rack: [number, number, number] = [-2.5, 0.9, -1.5];
  layout.rackBack.push({ position: rack, scale: [1.5, 1.8, 0.1] });
  for (const y of [0.5, 0, -0.5]) {
    layout.rackShelves.push({ position: [rack[0], rack[1] + y, rack[2] + 0.1], scale: [1.4, 0.06, 0.25] });
  }
  [-0.4, -0.1, 0.2, 0.5].forEach((x, i) => {
    const target = i % 2 === 0 ? layout.toolsCyan : layout.toolsAmber;
    target.push({ position: [rack[0] + x, rack[1] + 0.6, rack[2] + 0.15], scale: [0.08, 0.2 + i * 0.05, 0.08] });
  });

  for (const [x, z] of [[0, 1.2], [1.5, 1.2]] as [number, number][]) {
    layout.stoolSeats.push({ position: [x, 0.65, z] });
    layout.stoolStems.push({ position: [x, 0.35, z] });
    layout.stoolBases.push({ position: [x, 0.05, z] });
  }
  return layout;
}

function LegacyHoloPulse() {
  useFrame(() => { legacyHoloPanel.opacity = 0.15 + 0.05 * Math.sin(performance.now() * 0.002); });
  return null;
}

function WorkbenchArea() {
  const layout = useMemo(buildWorkbenchLayout, []);
  return (
    <group position={[0, 0, -10]}>
      <InstancedProp name="legacy.workbench.tops" geometry={unitBox} material={legacyDarkStone} transforms={layout.tops} castShadow />
      <InstancedProp name="legacy.workbench.legs" geometry={unitBox} material={legacyWood} transforms={layout.legs} castShadow />
      <InstancedProp name="legacy.workbench.drawers" geometry={unitBox} material={legacyWood} transforms={layout.drawers} />
      <InstancedProp name="legacy.workbench.handles" geometry={unitBox} material={legacyHandle} transforms={layout.handles} />
      <InstancedProp name="legacy.workbench.circuits" geometry={legacyCircuitGeo} material={legacyCircuit} transforms={layout.circuits} />
      <InstancedProp name="legacy.workbench.panels" geometry={unitPlane} material={legacyHoloPanel} transforms={layout.panels} />
      <InstancedProp name="legacy.workbench.frames" geometry={unitBox} material={legacyHoloFrame} transforms={layout.frames} />
      <InstancedProp name="legacy.workbench.lines" geometry={unitBox} material={legacyHoloLine} transforms={layout.lines} />
      <InstancedProp name="legacy.workbench.rackBack" geometry={unitBox} material={legacyDarkStone} transforms={layout.rackBack} />
      <InstancedProp name="legacy.workbench.rackShelves" geometry={unitBox} material={legacyDarkStone} transforms={layout.rackShelves} />
      <InstancedProp name="legacy.workbench.tools.cyan" geometry={legacyToolGeo} material={legacyToolCyan} transforms={layout.toolsCyan} />
      <InstancedProp name="legacy.workbench.tools.amber" geometry={legacyToolGeo} material={legacyToolAmber} transforms={layout.toolsAmber} />
      <InstancedProp name="legacy.workbench.stoolSeats" geometry={legacyStoolSeatGeo} material={legacyStoolSeat} transforms={layout.stoolSeats} castShadow />
      <InstancedProp name="legacy.workbench.stoolStems" geometry={legacyStoolStemGeo} material={legacyMetal} transforms={layout.stoolStems} />
      <InstancedProp name="legacy.workbench.stoolBases" geometry={legacyStoolBaseGeo} material={legacyMetal} transforms={layout.stoolBases} />
      <LegacyHoloPulse />
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
        <meshLambertMaterial color="#F5F0E0" />
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
        {/* A 3-unit desk lamp pool. Every point light in a forward renderer
            is paid for by every lit fragment on screen, so a lamp this small
            is the worst possible trade. The fixture still reads as a lamp. */}
      </group>
    </group>
  );
}

/** Static replacement for the six mounted ReadingDesk instances. */
export function BatchedReadingDesks({ positions }: { positions: readonly [number, number, number][] }) {
  const marble = useMarbleMat();
  const wood = useWoodMat();
  const desktops: InstanceTransform[] = [];
  const legs: InstanceTransform[] = [];
  const books: InstanceTransform[] = [];
  const lampBases: InstanceTransform[] = [];
  const lampStems: InstanceTransform[] = [];
  const lampShades: InstanceTransform[] = [];
  for (const [x, y, z] of positions) {
    desktops.push({ position: [x, y + 0.8, z], scale: [1.8, 0.1, 1] });
    for (const [lx, lz] of [[-0.7, -0.35], [-0.7, 0.35], [0.7, -0.35], [0.7, 0.35]] as [number, number][]) {
      legs.push({ position: [x + lx, y + 0.4, z + lz] });
    }
    books.push({ position: [x, y + 0.87, z], rotation: [-0.1, 0, 0], scale: [0.5, 0.02, 0.35] });
    lampBases.push({ position: [x + 0.7, y + 0.85, z - 0.3] });
    lampStems.push({ position: [x + 0.7, y + 1.15, z - 0.3] });
    lampShades.push({ position: [x + 0.7, y + 1.4, z - 0.25], rotation: [0.3, 0, 0] });
  }
  return (
    <>
      <InstancedProp name="mezz.readingDesk.desktop" geometry={unitBox} material={marble} transforms={desktops} castShadow />
      <InstancedProp name="mezz.readingDesk.legs" geometry={legacyDeskLegGeo} material={wood} transforms={legs} />
      <InstancedProp name="mezz.readingDesk.books" geometry={unitBox} material={legacyDeskBookMat} transforms={books} />
      <InstancedProp name="mezz.readingDesk.lampBase" geometry={legacyDeskLampBaseGeo} material={legacyDeskLampMat} transforms={lampBases} />
      <InstancedProp name="mezz.readingDesk.lampStem" geometry={legacyDeskLampStemGeo} material={legacyDeskLampMat} transforms={lampStems} />
      <InstancedProp name="mezz.readingDesk.lampShade" geometry={legacyDeskLampShadeGeo} material={legacyDeskLampShadeMat} transforms={lampShades} />
    </>
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
        <meshLambertMaterial color={COLORS.primaryMarble} />
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
        {/* Same trade as the desk lamp: the emissive orb above is the whole
            accent, the 3-unit light was just a glow on the base. */}
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
// Lounge with couches and low table. The portal itself is mounted by Zones.

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
        <meshLambertMaterial color={COLORS.primaryMarble} />
      </mesh>
      {/* Left pillar */}
      <mesh position={[-1.5, 1, 0]}>
        <cylinderGeometry args={[0.15, 0.18, 2, 8]} />
        <meshLambertMaterial color={COLORS.primaryMarble} />
      </mesh>
      {/* Right pillar */}
      <mesh position={[1.5, 1, 0]}>
        <cylinderGeometry args={[0.15, 0.18, 2, 8]} />
        <meshLambertMaterial color={COLORS.primaryMarble} />
      </mesh>
      {/* Swirling vortex inside */}
      <mesh ref={ref} position-y={1.8}>
        <torusGeometry args={[1, 0.3, 8, 32]} />
        <meshBasicMaterial color={COLORS.neonAmber} transparent opacity={0.2} depthWrite={false} />
      </mesh>
      {/* The vortex torus above is additive and carries the inner glow. */}
    </group>
  );
}

// === SHARED FURNITURE PIECES ===

interface ForumLayout {
  couchSeats: InstanceTransform[];
  couchBacks: InstanceTransform[];
  couchArms: InstanceTransform[];
  couchLegs: InstanceTransform[];
  couchTrims: InstanceTransform[];
  tableTops: InstanceTransform[];
  tablePillars: InstanceTransform[];
  tableBases: InstanceTransform[];
}

function forumPoint(
  origin: [number, number, number],
  rotation: number,
  local: [number, number, number],
): [number, number, number] {
  const c = Math.cos(rotation);
  const s = Math.sin(rotation);
  return [origin[0] + c * local[0] + s * local[2], origin[1] + local[1], origin[2] - s * local[0] + c * local[2]];
}

export function buildForumLayout(): ForumLayout {
  const layout: ForumLayout = {
    couchSeats: [], couchBacks: [], couchArms: [], couchLegs: [], couchTrims: [],
    tableTops: [], tablePillars: [], tableBases: [],
  };
  const couches: [number, number, number, number][] = [[-3, 0, 3, -0.4], [0, 0, 4, 0], [3, 0, 3, 0.4]];
  for (const [x, y, z, rotation] of couches) {
    const origin: [number, number, number] = [x, y, z];
    const r = [0, rotation, 0] as [number, number, number];
    layout.couchSeats.push({ position: forumPoint(origin, rotation, [0, 0.35, 0]), rotation: r, scale: [1.8, 0.3, 0.7] });
    layout.couchBacks.push({ position: forumPoint(origin, rotation, [0, 0.65, -0.3]), rotation: r, scale: [1.8, 0.5, 0.15] });
    for (const xArm of [-0.85, 0.85]) {
      layout.couchArms.push({ position: forumPoint(origin, rotation, [xArm, 0.5, 0]), rotation: r, scale: [0.12, 0.35, 0.7] });
    }
    for (const [xLeg, zLeg] of [[-0.75, -0.25], [-0.75, 0.25], [0.75, -0.25], [0.75, 0.25]] as [number, number][]) {
      layout.couchLegs.push({ position: forumPoint(origin, rotation, [xLeg, 0.1, zLeg]), rotation: r, scale: [0.06, 0.2, 0.06] });
    }
    layout.couchTrims.push({ position: forumPoint(origin, rotation, [0, 0.21, 0.35]), rotation: r, scale: [1.75, 0.02, 0.02] });
  }
  const table: [number, number, number] = [0, 0, 3];
  layout.tableTops.push({ position: [table[0], 0.4, table[2]] });
  layout.tablePillars.push({ position: [table[0], 0.2, table[2]] });
  layout.tableBases.push({ position: [table[0], 0.03, table[2]] });
  return layout;
}

function ForumArea() {
  const layout = useMemo(buildForumLayout, []);
  const marble = useMarbleMat();
  return (
    <group position={[-10, 0, 0]} rotation-y={Math.PI / 2}>
      <InstancedProp name="legacy.forum.couchSeats" geometry={unitBox} material={legacyCouchFabric} transforms={layout.couchSeats} castShadow />
      <InstancedProp name="legacy.forum.couchBacks" geometry={unitBox} material={legacyCouchFabric} transforms={layout.couchBacks} castShadow />
      <InstancedProp name="legacy.forum.couchArms" geometry={unitBox} material={legacyCouchFabric} transforms={layout.couchArms} />
      <InstancedProp name="legacy.forum.couchLegs" geometry={legacyCouchLegGeo} material={legacyCouchLeg} transforms={layout.couchLegs} />
      <InstancedProp name="legacy.forum.couchTrims" geometry={legacyCouchTrimGeo} material={legacyCouchTrim} transforms={layout.couchTrims} />
      <InstancedProp name="legacy.forum.tableTops" geometry={legacyLowTableTopGeo} material={marble} transforms={layout.tableTops} castShadow />
      <InstancedProp name="legacy.forum.tablePillars" geometry={legacyLowTablePillarGeo} material={marble} transforms={layout.tablePillars} />
      <InstancedProp name="legacy.forum.tableBases" geometry={legacyLowTableBaseGeo} material={marble} transforms={layout.tableBases} />
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
          <meshLambertMaterial color="#5C4033" />
        </mesh>
      ))}
    </group>
  );
}

/** Static replacement for the two mounted ArmChair instances. */
export function BatchedArmChairs({ chairs }: { chairs: readonly { position: [number, number, number]; rotation?: number }[] }) {
  const seats: InstanceTransform[] = [];
  const backs: InstanceTransform[] = [];
  const arms: InstanceTransform[] = [];
  const legs: InstanceTransform[] = [];
  for (const chair of chairs) {
    const rotation = chair.rotation ?? 0;
    const r = [0, rotation, 0] as [number, number, number];
    const c = Math.cos(rotation);
    const s = Math.sin(rotation);
    const point = (local: [number, number, number]): [number, number, number] => [
      chair.position[0] + c * local[0] + s * local[2],
      chair.position[1] + local[1],
      chair.position[2] - s * local[0] + c * local[2],
    ];
    seats.push({ position: point([0, 0.35, 0]), rotation: r, scale: [0.8, 0.25, 0.7] });
    backs.push({ position: point([0, 0.65, -0.3]), rotation: r, scale: [0.8, 0.5, 0.15] });
    for (const x of [-0.38, 0.38]) arms.push({ position: point([x, 0.5, 0]), rotation: r, scale: [0.1, 0.25, 0.6] });
    for (const [x, z] of [[-0.3, -0.25], [-0.3, 0.25], [0.3, -0.25], [0.3, 0.25]] as [number, number][]) {
      legs.push({ position: point([x, 0.1, z]), rotation: r, scale: [0.06, 0.2, 0.06] });
    }
  }
  return (
    <>
      <InstancedProp name="mezz.armchair.seats" geometry={unitBox} material={legacyArmchairFabric} transforms={seats} castShadow />
      <InstancedProp name="mezz.armchair.backs" geometry={unitBox} material={legacyArmchairFabric} transforms={backs} castShadow />
      <InstancedProp name="mezz.armchair.arms" geometry={unitBox} material={legacyArmchairFabric} transforms={arms} />
      <InstancedProp name="mezz.armchair.legs" geometry={legacyCouchLegGeo} material={legacyArmchairLeg} transforms={legs} />
    </>
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

// === MAIN EXPORT ===

export function LegacyFurniture() {
  return (
    <group>
      <WorkbenchArea />
      <ForumArea />
    </group>
  );
}
