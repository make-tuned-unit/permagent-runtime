// Mezzanine Library (The Brain) — part of the main hall per WORLD_VIEW_BIBLE.md §3.
// Moved verbatim from WorldFurniture.tsx in the bible §5 skeleton split (W1).
// Raised ring walkway high on the columns with built-in bookshelf walls.

import { useMemo } from 'react';
import * as THREE from 'three';
import { COLORS } from '../../constants';
import {
  useMarbleMat,
  useDarkStoneMat,
  BatchedReadingDesks,
  BatchedArmChairs,
} from '../../props/legacy/WorldFurniture';
import { MezzanineBookWall } from '../../props/MezzanineBookWall';
import { InstancedProp, type InstanceTransform } from '../../shared/instancing';
import { unitBox } from '../../props/geometries';

export const MEZZ_HEIGHT = 10;       // raised high so ground floor breathes
export const MEZZ_INNER_R = 12.5;
export const MEZZ_OUTER_R = 15.5;
const MEZZ_MID_R = (MEZZ_INNER_R + MEZZ_OUTER_R) / 2;

const STAIR_GAP_CENTER = Math.PI * 0.375; // between columns 1 and 2 (67.5 deg)
const STAIR_GAP_HALF = 0.12;           // small opening, ~3.4 units of arc at r=14
const STAIR_STEP_COUNT = 30;
const STAIR_RAIL_STEP = 3;
const stairNosingMat = new THREE.MeshLambertMaterial({ color: '#6d7482' });
const stairRailMat = new THREE.MeshStandardMaterial({ color: '#888', metalness: 0.5, roughness: 0.3 });
const stairRailPostGeo = new THREE.CylinderGeometry(0.5, 0.5, 1, 4);
const sconceCoreGeo = new THREE.SphereGeometry(0.06, 8, 8);
const sconceHaloGeo = new THREE.SphereGeometry(0.18, 12, 12);
const sconceCoreMat = new THREE.MeshBasicMaterial({ color: COLORS.neonAmber, toneMapped: false });
const sconceHaloMat = new THREE.MeshBasicMaterial({
  color: COLORS.neonAmber,
  transparent: true,
  opacity: 0.22,
  depthWrite: false,
  blending: THREE.AdditiveBlending,
  toneMapped: false,
});

// Walkable spiral-stair descriptor — shared with agent behavior so an agent's climb
// path matches the rendered steps exactly. The stair winds from the ground (t=0, at
// gapCenter - arcSpan) up to the mezzanine floor (t=1, at gapCenter, y=height).
export const STAIR = {
  gapCenter: STAIR_GAP_CENTER,
  arcSpan: Math.PI * 0.6,
  radius: MEZZ_MID_R,
  height: MEZZ_HEIGHT,
} as const;

/** World-space point on the stair centerline at climb fraction t∈[0,1]. */
export function stairPointAt(t: number): { x: number; y: number; z: number } {
  const angle = STAIR.gapCenter - STAIR.arcSpan * (1 - t);
  return { x: Math.cos(angle) * STAIR.radius, y: t * STAIR.height, z: Math.sin(angle) * STAIR.radius };
}

function isInStairGap(angle: number): boolean {
  let diff = angle - STAIR_GAP_CENTER;
  while (diff > Math.PI) diff -= Math.PI * 2;
  while (diff < -Math.PI) diff += Math.PI * 2;
  return Math.abs(diff) < STAIR_GAP_HALF;
}

/** Static staircase transforms; kept pure so the stair/anchor contract is testable. */
export function buildStaircaseLayout(): {
  steps: InstanceTransform[];
  nosings: InstanceTransform[];
  railPosts: InstanceTransform[];
} {
  const steps: InstanceTransform[] = [];
  const nosings: InstanceTransform[] = [];
  const railPosts: InstanceTransform[] = [];
  const endAngle = STAIR_GAP_CENTER;
  const arcSpan = STAIR.arcSpan;
  const startAngle = endAngle - arcSpan;
  const stairR = STAIR.radius;
  const stepWidth = MEZZ_OUTER_R - MEZZ_INNER_R - 0.4;

  for (let i = 0; i < STAIR_STEP_COUNT; i++) {
    const t = i / (STAIR_STEP_COUNT - 1);
    const angle = startAngle + t * arcSpan;
    const y = t * MEZZ_HEIGHT;
    const x = Math.cos(angle) * stairR;
    const z = Math.sin(angle) * stairR;
    const rotationY = -angle + Math.PI / 2;
    steps.push({
      position: [x, y + 0.05, z],
      rotation: [0, rotationY, 0],
      scale: [stepWidth, 0.15, 0.6],
    });
    // The nosing used to be a child mesh at local [0, .055, .31]. Resolve that
    // offset here so instancing preserves the exact world transform.
    nosings.push({
      position: [x + Math.cos(angle) * 0.31, y + 0.105, z + Math.sin(angle) * 0.31],
      rotation: [0, rotationY, 0],
      scale: [stepWidth, 0.05, 0.05],
    });
  }

  for (let i = 0; i < STAIR_STEP_COUNT; i++) {
    if (i % STAIR_RAIL_STEP !== 0) continue;
    const t = i / (STAIR_STEP_COUNT - 1);
    const angle = startAngle + t * arcSpan;
    const y = t * MEZZ_HEIGHT;
    railPosts.push(
      { position: [Math.cos(angle) * (MEZZ_INNER_R + 0.15), y + 0.5, Math.sin(angle) * (MEZZ_INNER_R + 0.15)], scale: [0.08, 1, 0.08] },
      { position: [Math.cos(angle) * (MEZZ_OUTER_R - 0.15), y + 0.5, Math.sin(angle) * (MEZZ_OUTER_R - 0.15)], scale: [0.08, 1, 0.08] },
    );
  }

  return { steps, nosings, railPosts };
}

/** Static outer-ring baluster transforms; the stair opening remains untouched. */
export function buildMezzanineBalusterLayout(): InstanceTransform[] {
  const transforms: InstanceTransform[] = [];
  for (let i = 0; i < 48; i++) {
    const angle = (i / 48) * Math.PI * 2;
    if (isInStairGap(angle)) continue;
    transforms.push({
      position: [Math.cos(angle) * MEZZ_OUTER_R, 0.4, Math.sin(angle) * MEZZ_OUTER_R],
      scale: [0.08, 0.8, 0.08],
    });
  }
  return transforms;
}

// RingGeometry lives in XY, rotated -PI/2 to XZ. This negates the Z mapping.
// To place a gap at world angle A, use ring angle = (2PI - A).
function ringAngle(worldA: number): number {
  return ((2 * Math.PI - worldA) % (2 * Math.PI) + 2 * Math.PI) % (2 * Math.PI);
}

function MezzanineRing() {
  const darkStone = useDarkStoneMat();
  const marble = useMarbleMat();
  const rGapCenter = ringAngle(STAIR_GAP_CENTER);
  const rStart = rGapCenter + STAIR_GAP_HALF;
  const rLength = Math.PI * 2 - STAIR_GAP_HALF * 2;
  const balusters = useMemo(buildMezzanineBalusterLayout, []);

  return (
    <group position-y={MEZZ_HEIGHT}>
      {/* Inner half of ring floor — CONTINUOUS, no gap (bookshelf wall sits on this) */}
      <mesh rotation-x={-Math.PI / 2} receiveShadow material={marble}>
        <ringGeometry args={[MEZZ_INNER_R, MEZZ_MID_R, 64, 1, 0, Math.PI * 2]} />
      </mesh>
      {/* Outer half of ring floor — has small stair gap */}
      <mesh rotation-x={-Math.PI / 2} receiveShadow material={marble}>
        <ringGeometry args={[MEZZ_MID_R, MEZZ_OUTER_R, 64, 1, rStart, rLength]} />
      </mesh>

      {/* Outer railing — one instanced draw, with the stair gap preserved. */}
      <InstancedProp name="mezz.outerBaluster" geometry={stairRailPostGeo} material={darkStone} transforms={balusters} />
      <mesh position-y={0.65} rotation-x={-Math.PI / 2}>
        <ringGeometry args={[MEZZ_OUTER_R - 0.03, MEZZ_OUTER_R + 0.03, 64, 1, rStart, rLength]} />
        <primitive object={darkStone} attach="material" />
      </mesh>
      {/* Rounded handrail cap on top of the balusters */}
      <mesh position-y={0.82} rotation-x={Math.PI / 2}>
        <torusGeometry args={[MEZZ_OUTER_R, 0.05, 6, 64]} />
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
  const layout = useMemo(buildStaircaseLayout, []);
  const endAngle = STAIR_GAP_CENTER;
  const arcSpan = STAIR.arcSpan; // wider spiral arc (shared with agent climb path)
  const startAngle = endAngle - arcSpan;
  const stairR = STAIR.radius;
  const stepWidth = MEZZ_OUTER_R - MEZZ_INNER_R - 0.4;

  // Under-stair stringers (#16 detail pass): two helical tubes hugging the
  // step undersides at the inner and outer edges, plus a center spine — the
  // steps read as a built staircase instead of floating slabs. One TubeGeometry
  // each (3 draw calls total), computed once.
  const stringers = useMemo(() => {
    const mk = (radius: number, tube: number) => {
      const pts: THREE.Vector3[] = [];
      const N = 40;
      for (let i = 0; i <= N; i++) {
        const t = i / N;
        const a = startAngle + t * arcSpan;
        pts.push(new THREE.Vector3(Math.cos(a) * radius, t * MEZZ_HEIGHT - 0.12, Math.sin(a) * radius));
      }
      return new THREE.TubeGeometry(new THREE.CatmullRomCurve3(pts), 60, tube, 6, false);
    };
    return [
      mk(MEZZ_INNER_R + 0.35, 0.09),
      mk(MEZZ_OUTER_R - 0.55, 0.09),
      mk(stairR, 0.13),
    ];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <group>
      {/* Steps and nosings keep their exact transforms while collapsing to one draw each. */}
      <InstancedProp name="mezz.stairStep" geometry={unitBox} material={marble} transforms={layout.steps} castShadow receiveShadow />
      <InstancedProp name="mezz.stairNosing" geometry={unitBox} material={stairNosingMat} transforms={layout.nosings} />
      {/* Helical stringers under the steps. */}
      {stringers.map((geo, i) => (
        <mesh key={`s-${i}`} geometry={geo} material={marble} castShadow />
      ))}
      {/* Ground landing slab at the stair foot. */}
      <mesh
        position={[Math.cos(startAngle) * stairR, 0.06, Math.sin(startAngle) * stairR]}
        rotation-y={-startAngle + Math.PI / 2}
        material={marble}
        receiveShadow
      >
        <boxGeometry args={[stepWidth + 0.8, 0.12, 1.6]} />
      </mesh>
      {/* Railings on both sides */}
      <InstancedProp name="mezz.stairRailPost" geometry={stairRailPostGeo} material={stairRailMat} transforms={layout.railPosts} />
    </group>
  );
}

// The bookshelf wall is the instanced MezzanineBookWall (props/): the legacy
// per-mesh wall drew ~443 individual meshes (1 wall + planks + dividers +
// ~390 books); the instanced replacement renders the identical silhouette —
// same radii, height, stair gap, varied spines via the 8-tone bookRamp — in
// 12 draw calls. This was the last big un-instanced mass in the hall.

function MezzanineLibraryContents() {
  const deskPositions = useMemo(() => {
    const positions: [number, number, number][] = [];
    for (let i = 0; i < 6; i++) {
      const angle = (i / 6) * Math.PI * 2 + Math.PI / 6;
      if (isInStairGap(angle)) continue;
      const r = MEZZ_INNER_R + 1.2;
      positions.push([Math.cos(angle) * r, 0, Math.sin(angle) * r]);
    }
    return positions;
  }, []);
  const sconceTransforms = useMemo<InstanceTransform[]>(() =>
    Array.from({ length: 8 }, (_, i) => {
      const angle = (i / 8) * Math.PI * 2;
      return { position: [Math.cos(angle) * MEZZ_MID_R, 2.5, Math.sin(angle) * MEZZ_MID_R] };
    }), []);
  return (
    <group position-y={MEZZ_HEIGHT + 0.15}>
      {/* Reading desks along inner walkway */}
      <BatchedReadingDesks positions={deskPositions} />
      {/* Armchairs */}
      <BatchedArmChairs chairs={[
        { position: [Math.cos(Math.PI) * MEZZ_MID_R, 0, Math.sin(Math.PI) * MEZZ_MID_R], rotation: 0 },
        { position: [Math.cos(0) * MEZZ_MID_R, 0, Math.sin(0) * MEZZ_MID_R], rotation: Math.PI },
      ]} />
      {/* Warm amber ring sconces. Light-census reduction (integration §1): these
          were 8 decorative pointLights — a pure rim array contributing 8 of the
          scene's 20 point lights for a barely-perceptible 0.2-intensity wash. They
          are now emissive glow fixtures (additive core + faint halo): the same warm
          dotted-ring read, at zero per-pixel light cost. The hall's real fill comes
          from the directional pair + the Mesh portal key. */}
      <InstancedProp name="mezz.sconce.core" geometry={sconceCoreGeo} material={sconceCoreMat} transforms={sconceTransforms} />
      <InstancedProp name="mezz.sconce.halo" geometry={sconceHaloGeo} material={sconceHaloMat} transforms={sconceTransforms} />
    </group>
  );
}

export function MezzanineLibrary() {
  return (
    <group>
      <MezzanineRing />
      <Staircase />
      <MezzanineBookWall />
      <MezzanineLibraryContents />
    </group>
  );
}
