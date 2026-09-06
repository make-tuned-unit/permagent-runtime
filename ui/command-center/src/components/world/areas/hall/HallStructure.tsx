// Main hall structure — platform, rotunda floor + circuits, columns, dome,
// station pedestals (threshold markers), orbital arcs, light shaft.
// Moved verbatim from WorldScene.tsx in the bible §5 skeleton split (W1).

import { useLayoutEffect, useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import { Float } from '@react-three/drei';
import * as THREE from 'three';
import { COLORS, STATIONS, COLUMN_COUNT, ROTUNDA_RADIUS, DOME_HEIGHT, PLATFORM_RADIUS } from '../../constants';
import { isPunchedAngle } from '../zones';
import { HallDetail } from './HallDetail';
import { HallInlay } from './HallInlay';
import { TaskDais } from './TaskDais';
import { BlenderVault } from './BlenderVault';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
// W4 reactivity seam (bible §7): the colonnade veins brighten with the live
// working-agent count. The driving signal stays in the atmosphere lane; this is
// the one cross-lane read W4 flagged in its PR for W1 awareness.
import { getVeinOpacity } from '../../atmosphere/ambience';
import { makeStoneTexture } from '../../shared/stoneTexture';

// Procedural stone floor texture (#16 realism pass): speckle noise + faint
// veining + BAKED radial ambient occlusion (dark rim under the colonnade,
// soft center shadow) — kills the flat blockout-gray read for one 512px
// canvas, no asset files, no extra draw calls.
function makeFloorTexture(): THREE.CanvasTexture {
  const size = 512;
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const ctx = c.getContext('2d')!;
  ctx.fillStyle = '#98a0ae';
  ctx.fillRect(0, 0, size, size);
  const img = ctx.getImageData(0, 0, size, size);
  for (let i = 0; i < img.data.length; i += 4) {
    const n = (Math.random() - 0.5) * 16;
    img.data[i] += n; img.data[i + 1] += n; img.data[i + 2] += n;
  }
  ctx.putImageData(img, 0, 0);
  // Faint marble veins
  ctx.globalAlpha = 0.055;
  ctx.strokeStyle = '#ffffff';
  for (let i = 0; i < 26; i++) {
    const x = Math.random() * size, y = Math.random() * size;
    ctx.beginPath();
    ctx.moveTo(x, y);
    ctx.bezierCurveTo(x + 70, y + 24, x + 90, y - 36, x + 180, y + 12);
    ctx.lineWidth = 1 + Math.random() * 1.6;
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
  // Baked AO: soft center shadow (under the shaft) + heavy rim (colonnade)
  const g = ctx.createRadialGradient(size / 2, size / 2, size * 0.16, size / 2, size / 2, size * 0.5);
  g.addColorStop(0, 'rgba(0,0,0,0.10)');
  g.addColorStop(0.55, 'rgba(0,0,0,0)');
  g.addColorStop(0.86, 'rgba(0,0,0,0.16)');
  g.addColorStop(1, 'rgba(0,0,0,0.42)');
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, size, size);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

// Floor with glowing circuit mandala pattern
function RotundaFloor() {
  const floorMaterial = useMemo(() => {
    const map = makeFloorTexture();
    return new THREE.MeshLambertMaterial({
      map,
      polygonOffset: true,
      polygonOffsetFactor: 1,
      polygonOffsetUnits: 1,
    });
  }, []);

  return (
    <group position-y={0.05}>
      {/* Main marble floor — raised above platform to avoid z-fighting */}
      <mesh rotation-x={-Math.PI / 2} receiveShadow>
        <circleGeometry args={[ROTUNDA_RADIUS, 64]} />
        <primitive object={floorMaterial} attach="material" />
      </mesh>
      {/* The warm bounce off the shelf glow above used to be two 20-unit
          pointLights standing right here. It is now one hemisphereLight in
          atmosphere/Atmosphere.tsx: a bounce from above is exactly what a
          hemisphere light is for, it covers the whole rotunda instead of two
          spots, and a forward renderer charges for it once rather than twice
          per fragment on every lit surface in the scene. */}
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
      const now = performance.now();
      groupRef.current.children.forEach((col, i) => {
        const vein = col.children[1];
        if (vein instanceof THREE.Mesh) {
          const mat = vein.material as THREE.MeshBasicMaterial;
          // W4 reactivity: amplitude scales with the live working count
          // (getVeinOpacity, ≤ 1.5× idle per bible §7). reduceMotion pins the
          // ambience level to idle, so this stays calm with no extra branch.
          mat.opacity = getVeinOpacity(i, now);
        }
      });
    }
  });

  // The colonnade is a full unbroken ring except the single opening where the
  // Mesh Stargate stands (isPunchedAngle — only the antechamber axis now).
  const columns = useMemo(() => {
    return Array.from({ length: COLUMN_COUNT }, (_, i) => {
      const angle = (i / COLUMN_COUNT) * Math.PI * 2;
      const x = Math.cos(angle) * (ROTUNDA_RADIUS - 1);
      const z = Math.sin(angle) * (ROTUNDA_RADIUS - 1);
      return { x, z, angle };
    }).filter(({ angle }) => !isPunchedAngle(angle));
  }, []);

  // Shared textured materials (#16): fluted shaft + plain stone trim, one
  // material each across the whole colonnade.
  const bodyMat = useMemo(() => {
    const map = makeStoneTexture('#aeb4c0', 12);
    map.repeat.set(1, 2.5);
    return new THREE.MeshLambertMaterial({ map });
  }, []);
  const trimMat = useMemo(
    () => new THREE.MeshLambertMaterial({ map: makeStoneTexture('#b6bcc8') }),
    []
  );

  return (
    <group ref={groupRef}>
      {columns.map(({ x, z }, i) => (
        <group key={i} position={[x, 0, z]}>
          {/* Column body */}
          <mesh castShadow position-y={DOME_HEIGHT / 2}>
            <cylinderGeometry args={[0.6, 0.7, DOME_HEIGHT, 16]} />
            <primitive object={bodyMat} attach="material" />
          </mesh>
          {/* Circuit vein */}
          <mesh position-y={DOME_HEIGHT / 2}>
            <cylinderGeometry args={[0.15, 0.15, DOME_HEIGHT - 1, 8]} />
            <meshBasicMaterial color={COLORS.neonCyan} transparent opacity={0.7} />
          </mesh>
          {/* Capital (top) */}
          <mesh position-y={DOME_HEIGHT + 0.3}>
            <cylinderGeometry args={[0.9, 0.6, 0.6, 16]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
          {/* Base */}
          <mesh position-y={0.3}>
            <cylinderGeometry args={[0.7, 0.9, 0.6, 16]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
          {/* Rim glow at the column edge — subtle cyan wash. Light-census reduction
              (integration §1): this was a 0.15-intensity decorative pointLight, one
              per surviving column (3 of the scene's 20 point lights). The column
              already carries an emissive cyan circuit-vein mesh above; this faint
              additive sprite reproduces the edge wash with zero light cost. */}
          <mesh position={[0.7, DOME_HEIGHT * 0.7, 0]}>
            <sphereGeometry args={[0.5, 10, 10]} />
            <meshBasicMaterial
              color={COLORS.neonCyan}
              transparent
              opacity={0.12}
              depthWrite={false}
              blending={THREE.AdditiveBlending}
              toneMapped={false}
            />
          </mesh>
        </group>
      ))}
    </group>
  );
}

// Dome ceiling with oculus
function Dome() {
  const shellMat = useMemo(() => {
    const map = makeStoneTexture('#a8aebb');
    map.repeat.set(6, 3);
    return new THREE.MeshLambertMaterial({ map, side: THREE.BackSide });
  }, []);
  return (
    <group>
      {/* Dome shell */}
      <mesh position-y={DOME_HEIGHT}>
        <sphereGeometry args={[ROTUNDA_RADIUS + 1, 32, 16, 0, Math.PI * 2, 0, Math.PI / 2]} />
        <primitive object={shellMat} attach="material" />
      </mesh>
      {/* Oculus ring */}
      <mesh position-y={DOME_HEIGHT + ROTUNDA_RADIUS * 0.97} rotation-x={Math.PI / 2}>
        <ringGeometry args={[2, 3, 32]} />
        <meshLambertMaterial color={COLORS.marbleVeining} />
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
  const layout = useMemo(buildStationPedestalLayout, []);
  const stoneMat = useMemo(
    () => new THREE.MeshLambertMaterial({ map: makeStoneTexture('#aeb4c0', 6) }),
    []
  );
  const plinthMat = useMemo(
    () => new THREE.MeshLambertMaterial({ map: makeStoneTexture('#8e94a2') }),
    []
  );
  const ringMat = useMemo(
    () => new THREE.MeshBasicMaterial({ color: COLORS.neonCyan, transparent: true, opacity: 0.5 }),
    []
  );

  return (
    <group>
      <InstancedProp
        name="hall.station.pedestals"
        geometry={layout.pedestalGeometry}
        material={stoneMat}
        transforms={layout.pedestals}
        castShadow
      />
      <InstancedProp
        name="hall.station.plinths"
        geometry={layout.plinthGeometry}
        material={plinthMat}
        transforms={layout.plinths}
      />
      <InstancedProp
        name="hall.station.caps"
        geometry={layout.capGeometry}
        material={plinthMat}
        transforms={layout.caps}
      />
      <InstancedProp
        name="hall.station.rings"
        geometry={layout.ringGeometry}
        material={ringMat}
        transforms={layout.rings}
      />
      <StationInteractionTargets
        geometry={layout.interactionGeometry}
        transforms={layout.interactionTargets}
        onHover={onHoverStation}
        onClick={onClickStation}
      />
      {STATIONS.map((station) => (
        <StationPedestal
          key={station.id}
          station={station}
        />
      ))}
    </group>
  );
}

interface StationPedestalLayout {
  ids: string[];
  pedestals: InstanceTransform[];
  plinths: InstanceTransform[];
  caps: InstanceTransform[];
  rings: InstanceTransform[];
  interactionTargets: InstanceTransform[];
  pedestalGeometry: THREE.CylinderGeometry;
  plinthGeometry: THREE.CylinderGeometry;
  capGeometry: THREE.CylinderGeometry;
  ringGeometry: THREE.RingGeometry;
  interactionGeometry: THREE.CylinderGeometry;
}

/** Static station transforms stay separate from interaction identity. */
export function buildStationPedestalLayout(): StationPedestalLayout {
  const pedestalHeight = 1.5;
  return {
    ids: STATIONS.map((station) => station.id),
    pedestals: STATIONS.map(({ position }) => ({ position: [position[0], pedestalHeight / 2, position[2]] })),
    plinths: STATIONS.map(({ position }) => ({ position: [position[0], 0.1, position[2]] })),
    caps: STATIONS.map(({ position }) => ({ position: [position[0], pedestalHeight + 0.06, position[2]] })),
    rings: STATIONS.map(({ position }) => ({
      position: [position[0], pedestalHeight + 0.01, position[2]],
      rotation: [-Math.PI / 2, 0, 0],
    })),
    interactionTargets: STATIONS.map(({ position }) => ({ position: [position[0], 1.4, position[2]] })),
    pedestalGeometry: new THREE.CylinderGeometry(0.6, 0.8, pedestalHeight, 6),
    plinthGeometry: new THREE.CylinderGeometry(0.95, 1.05, 0.2, 6),
    capGeometry: new THREE.CylinderGeometry(0.78, 0.62, 0.14, 6),
    ringGeometry: new THREE.RingGeometry(0.4, 0.6, 32),
    // One transparent raycast volume covers the pedestal and floating icon,
    // retaining parent-group click/hover reach without four duplicate meshes.
    interactionGeometry: new THREE.CylinderGeometry(1.1, 1.1, 3.4, 8),
  };
}

export function stationIdForInstance(ids: readonly string[], instanceId: number | undefined): string | null {
  return instanceId === undefined || instanceId < 0 || instanceId >= ids.length ? null : ids[instanceId];
}

function StationInteractionTargets({
  geometry,
  transforms,
  onHover,
  onClick,
}: {
  geometry: THREE.CylinderGeometry;
  transforms: InstanceTransform[];
  onHover: (id: string | null) => void;
  onClick: (id: string) => void;
}) {
  const ref = useRef<THREE.InstancedMesh>(null);
  const material = useMemo(
    () => new THREE.MeshBasicMaterial({ transparent: true, opacity: 0, colorWrite: false, depthWrite: false }),
    []
  );
  useLayoutEffect(() => {
    const mesh = ref.current;
    if (!mesh) return;
    const object = new THREE.Object3D();
    transforms.forEach((transform, index) => {
      object.position.set(...transform.position);
      object.rotation.set(...(transform.rotation ?? [0, 0, 0]));
      object.scale.setScalar(1);
      object.updateMatrix();
      mesh.setMatrixAt(index, object.matrix);
    });
    mesh.instanceMatrix.needsUpdate = true;
  }, [transforms]);

  const stationIds = STATIONS.map((station) => station.id);
  return (
    <instancedMesh
      ref={ref}
      args={[geometry, material, transforms.length]}
      onPointerOver={(event) => {
        event.stopPropagation();
        const id = stationIdForInstance(stationIds, event.instanceId);
        if (id) {
          document.body.style.cursor = 'pointer';
          onHover(id);
        }
      }}
      onPointerOut={(event) => {
        event.stopPropagation();
        document.body.style.cursor = 'auto';
        onHover(null);
      }}
      onClick={(event) => {
        event.stopPropagation();
        const id = stationIdForInstance(stationIds, event.instanceId);
        if (id) onClick(id);
      }}
    />
  );
}

function StationPedestal({
  station,
}: {
  station: (typeof STATIONS)[number];
}) {
  const isPortal = station.iconType === 'portal';
  const pedestalHeight = isPortal ? 2 : 1.5;
  return (
    <group
      position={station.position}
    >
      {/* Floating icon */}
      <Float speed={2} rotationIntensity={0.3} floatIntensity={0.5}>
        <group position-y={pedestalHeight + 1.2}>
          <StationIcon type={station.iconType} />
        </group>
      </Float>
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
    case 'rings':
      // Horologium: concentric clock rings + a single tick at 12.
      return (
        <group>
          <mesh>
            <torusGeometry args={[0.42, 0.05, 8, 28]} />
            <meshBasicMaterial color={color} transparent opacity={0.8} />
          </mesh>
          <mesh>
            <torusGeometry args={[0.24, 0.04, 8, 20]} />
            <meshBasicMaterial color={color} transparent opacity={0.6} />
          </mesh>
          <mesh position={[0, 0.34, 0]}>
            <boxGeometry args={[0.05, 0.14, 0.05]} />
            <meshBasicMaterial color={color} transparent opacity={0.9} />
          </mesh>
        </group>
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
      {/* Only the static shell is Blender-authored. Live props, anchors,
          station navigation and agent/state animation remain unchanged. */}
      <BlenderVault fallback={<><Columns /><Dome /><HallDetail /></>} />
      {/* Engraved circuit-node inlay (instanced) + the omphalos: the rotunda's
          reactive heart, breathing with REAL Brain events (HallInlay.tsx). */}
      <HallInlay />
      {/* The task dais — agents step onto it when they pick up work and the
          beam transmits the task down into them (areas/hall/TaskDais.tsx). */}
      <TaskDais />
      <StationPedestals onHoverStation={onHoverStation} onClickStation={onClickStation} />

      {/* Orbital arcs — signature dynamic visual */}

      {/* Light shaft from oculus */}
    </>
  );
}
