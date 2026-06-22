// Mezzanine Library (The Brain) — part of the main hall per WORLD_VIEW_BIBLE.md §3.
// Moved verbatim from WorldFurniture.tsx in the bible §5 skeleton split (W1).
// Raised ring walkway high on the columns with built-in bookshelf walls.

import { useMemo } from 'react';
import * as THREE from 'three';
import { COLORS } from '../../constants';
import {
  useMarbleMat,
  useDarkStoneMat,
  useWoodMat,
  ReadingDesk,
  ArmChair,
} from '../../props/legacy/WorldFurniture';

export const MEZZ_HEIGHT = 10;       // raised high so ground floor breathes
export const MEZZ_INNER_R = 12.5;
export const MEZZ_OUTER_R = 15.5;
const MEZZ_MID_R = (MEZZ_INNER_R + MEZZ_OUTER_R) / 2;

const STAIR_GAP_CENTER = Math.PI * 0.375; // between columns 1 and 2 (67.5 deg)
const STAIR_GAP_HALF = 0.12;           // small opening, ~3.4 units of arc at r=14
const SHELF_WALL_HEIGHT = 4;

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
  const arcSpan = STAIR.arcSpan; // wider spiral arc (shared with agent climb path)
  const startAngle = endAngle - arcSpan;
  const stairR = STAIR.radius;
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
      {/* Warm amber ring sconces. Light-census reduction (integration §1): these
          were 8 decorative pointLights — a pure rim array contributing 8 of the
          scene's 20 point lights for a barely-perceptible 0.2-intensity wash. They
          are now emissive glow fixtures (additive core + faint halo): the same warm
          dotted-ring read, at zero per-pixel light cost. The hall's real fill comes
          from the §1 directional pair + the Mouth key. */}
      {Array.from({ length: 8 }, (_, i) => {
        const angle = (i / 8) * Math.PI * 2;
        const x = Math.cos(angle) * MEZZ_MID_R;
        const z = Math.sin(angle) * MEZZ_MID_R;
        return (
          <group key={i} position={[x, 2.5, z]}>
            {/* Bright emissive core — the sconce point itself */}
            <mesh>
              <sphereGeometry args={[0.06, 8, 8]} />
              <meshBasicMaterial color={COLORS.neonAmber} toneMapped={false} />
            </mesh>
            {/* Faint additive halo — the soft warm bloom the pointLight used to cast */}
            <mesh>
              <sphereGeometry args={[0.18, 12, 12]} />
              <meshBasicMaterial
                color={COLORS.neonAmber}
                transparent
                opacity={0.22}
                depthWrite={false}
                blending={THREE.AdditiveBlending}
                toneMapped={false}
              />
            </mesh>
          </group>
        );
      })}
    </group>
  );
}

export function MezzanineLibrary() {
  return (
    <group>
      <MezzanineRing />
      <Staircase />
      <BookshelfWall />
      <MezzanineLibraryContents />
    </group>
  );
}
