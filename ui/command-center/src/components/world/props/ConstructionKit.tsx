// W2 — ConstructionKit: the Cave's construction system (Cave §4).
// "Scaffolding is the most important visual state in the world": light-lattice
// + stone, slim amber work-lights, an idle crane at night, banked-stone piles
// where builders slow-bank material, and glowing survey markers laying out where
// future wings will rise. A new user's world is a construction site at first
// light — these are its load-bearing props.
//
// Composability (Cave §4 / W1 handoff): every sub-element is an independent,
// position/rotation-parameterised component so W1 can drop a Scaffold, a
// SurveyLine, or a StonePile at any per-chamber mount point (areas/strata
// anchors, once they land). Until then they compose from the dev catalog.
//
// Material grammar (Cave §3): stone = the given world (stoneRough); the lattice
// and survey lines are tending-register work-light (lightTending, NEVER HUD
// amber); the crane's task lamp is a slim amber WORK-light (lightAmberWork, also
// not HUD amber — see materials.ts). Light-veins/carved cuts that read as the
// agents' UNDERSTANDING stay cyan (lightCyan), reserved for finished carving.
//
// Instancing (bible §8): lattice struts, banked rocks, survey caps and stakes
// are all InstancedProp — a full scaffold tower is one draw call per material.
// Anchors: a Scaffold publishes 2 'lean' anchors (a worker leaning on the rail);
// a StonePile publishes 1 'stand' (a worker setting from the pile).

import { useMemo, useRef } from 'react';
import * as THREE from 'three';
import { useFrame } from '@react-three/fiber';
import { getReduceMotion } from '../../../styles/tokens';
import type { AreaId, AgentAnchor } from '../shared/anchors';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, unitCylinder8, unitRock, unitTetra, hash01 } from './geometries';
import {
  stoneRough,
  metalGunmetal,
  metalBronze,
  lightAmberWork,
  lightTending,
} from './materials';
import { placeAnchor, useRegisterAnchors } from './propUtils';

// ─── Scaffold: light-lattice + stone footings + slim amber work-lights ───────

interface ScaffoldProps {
  position?: [number, number, number];
  rotationY?: number;
  /** Bays wide (each ~2u) and levels tall (each ~2u). */
  bays?: number;
  levels?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

const BAY_W = 2.0;
const LEVEL_H = 2.0;

/**
 * Scaffold lattice. The defining prop of the early world: stone footings, a
 * light-lattice of slim struts (tending register), and amber work-lamps clamped
 * to the top rail. Reads at 25u as one dark lattice mass + a few warm pinpricks.
 */
export function Scaffold({
  position = [0, 0, 0],
  rotationY = 0,
  bays = 2,
  levels = 3,
  areaId = 'build',
  idPrefix = 'scaffold',
}: ScaffoldProps) {
  const layout = useMemo(() => {
    const footings: InstanceTransform[] = [];
    const uprights: InstanceTransform[] = [];
    const ledgers: InstanceTransform[] = []; // horizontal lattice members (lightTending)
    const braces: InstanceTransform[] = []; // diagonal lattice (lightTending)
    const lamps: InstanceTransform[] = []; // amber work-lights (lightAmberWork)

    const span = bays * BAY_W;
    const top = levels * LEVEL_H;
    const colX = Array.from({ length: bays + 1 }, (_, i) => -span / 2 + i * BAY_W);

    // Vertical uprights (gunmetal) on stone footings, one column per bay edge,
    // front (z=-0.5) and back (z=+0.5) plane.
    for (const x of colX) {
      for (const z of [-0.5, 0.5]) {
        footings.push({ position: [x, 0.15, z], scale: [0.5, 0.3, 0.5] });
        uprights.push({ position: [x, top / 2, z], scale: [0.12, top, 0.12] });
      }
    }

    // Horizontal ledgers (the glowing lattice rails) at every level, both planes.
    for (let lvl = 1; lvl <= levels; lvl++) {
      const y = lvl * LEVEL_H;
      for (const z of [-0.5, 0.5]) {
        ledgers.push({ position: [0, y, z], scale: [span, 0.07, 0.07] });
      }
      // Transverse ties front-to-back at each column.
      for (const x of colX) {
        ledgers.push({ position: [x, y, 0], scale: [0.07, 0.07, 1.0] });
      }
    }

    // Diagonal braces — one per bay, alternating direction per level (the lattice
    // "weave" that makes scaffolding read as scaffolding).
    const diagLen = Math.hypot(BAY_W, LEVEL_H);
    const diagAngle = Math.atan2(LEVEL_H, BAY_W);
    for (let b = 0; b < bays; b++) {
      const cx = -span / 2 + (b + 0.5) * BAY_W;
      for (let lvl = 0; lvl < levels; lvl++) {
        const cy = (lvl + 0.5) * LEVEL_H;
        const dir = (b + lvl) % 2 === 0 ? 1 : -1;
        braces.push({
          position: [cx, cy, -0.5],
          rotation: [0, 0, dir * diagAngle],
          scale: [diagLen, 0.05, 0.05],
        });
      }
    }

    // Slim amber work-lamps clamped along the top rail (front plane), every bay.
    for (let b = 0; b <= bays; b++) {
      const x = -span / 2 + b * BAY_W;
      lamps.push({ position: [x, top + 0.18, -0.5], scale: [0.16, 0.16, 0.16] });
    }

    return { footings, uprights, ledgers, braces, lamps };
  }, [bays, levels]);

  const anchors = useMemo<AgentAnchor[]>(() => {
    const span = bays * BAY_W;
    return [
      {
        id: `${idPrefix}.railL.lean`,
        areaId,
        kind: 'lean' as const,
        ...placeAnchor([-span / 2 + 0.4, 0, 0.9], 0, position, rotationY),
      },
      {
        id: `${idPrefix}.railR.lean`,
        areaId,
        kind: 'lean' as const,
        ...placeAnchor([span / 2 - 0.4, 0, 0.9], 0, position, rotationY),
      },
    ];
  }, [areaId, idPrefix, position, rotationY, bays]);
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      <InstancedProp name={`${idPrefix}.footing`} geometry={unitBox} material={stoneRough} transforms={layout.footings} castShadow receiveShadow />
      <InstancedProp name={`${idPrefix}.upright`} geometry={unitCylinder8} material={metalGunmetal} transforms={layout.uprights} castShadow />
      <InstancedProp name={`${idPrefix}.ledger`} geometry={unitBox} material={lightTending} transforms={layout.ledgers} />
      <InstancedProp name={`${idPrefix}.brace`} geometry={unitBox} material={lightTending} transforms={layout.braces} />
      <InstancedProp name={`${idPrefix}.lamp`} geometry={unitBox} material={lightAmberWork} transforms={layout.lamps} />
    </group>
  );
}

// ─── IdleCrane: the bronze gantry crane arm, parked at night ─────────────────

interface IdleCraneProps {
  position?: [number, number, number];
  rotationY?: number;
  /** Slow idle sway of the hanging hook block (Cave §4 "idle crane at night"). */
  animated?: boolean;
}

/**
 * The idle crane. A bronze mast, a long counter-weighted jib, a cable, and a
 * hook block that sways almost imperceptibly. One slim amber work-lamp at the
 * jib tip. The signature "construction site at first light" silhouette.
 * A unique focal singleton — no instancing, no anchors (W3 owns who operates it).
 */
export function IdleCrane({
  position = [0, 0, 0],
  rotationY = 0,
  animated = true,
}: IdleCraneProps) {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const hookRef = useRef<THREE.Group>(null);

  useFrame(({ clock }) => {
    if (!animated || reduceMotion || !hookRef.current) return;
    // Barely-there pendulum: ±2° on a slow ~7s period.
    hookRef.current.rotation.z = Math.sin(clock.elapsedTime * 0.9) * 0.035;
  });

  return (
    <group position={position} rotation-y={rotationY}>
      {/* Stone counterweight footing + bronze mast */}
      <mesh position={[0, 0.3, 0]} scale={[1.1, 0.6, 1.1]} geometry={unitBox} material={stoneRough} castShadow receiveShadow />
      <mesh position={[0, 3.0, 0]} scale={[0.4, 6.0, 0.4]} geometry={unitCylinder8} material={metalBronze} castShadow />
      {/* Jib reaching over the threshold (Cave §4 / bible A1 6u gantry) */}
      <mesh position={[2.2, 6.0, 0]} scale={[6.0, 0.3, 0.3]} geometry={unitBox} material={metalBronze} castShadow />
      {/* Counter-jib + counterweight block */}
      <mesh position={[-1.4, 6.0, 0]} scale={[2.0, 0.3, 0.3]} geometry={unitBox} material={metalBronze} />
      <mesh position={[-2.2, 5.7, 0]} scale={[0.7, 0.7, 0.7]} geometry={unitBox} material={stoneRough} castShadow />
      {/* Jib-tip amber work-lamp */}
      <mesh position={[5.0, 6.05, 0]} scale={[0.18, 0.18, 0.18]} geometry={unitBox} material={lightAmberWork} />
      {/* Cable + hook block, swaying group pivoted at the jib tip */}
      <group ref={hookRef} position={[5.0, 5.85, 0]}>
        <mesh position={[0, -1.0, 0]} scale={[0.03, 2.0, 0.03]} geometry={unitCylinder8} material={metalGunmetal} />
        <mesh position={[0, -2.1, 0]} scale={[0.35, 0.4, 0.35]} geometry={unitBox} material={metalGunmetal} castShadow />
      </group>
    </group>
  );
}

// ─── StonePile: banked quarried material (Cave §4 "builders bank material") ───

interface StonePileProps {
  position?: [number, number, number];
  rotationY?: number;
  /** Pile size: more rocks = a busier bank. */
  count?: number;
  /** Seed so a chamber's piles differ deterministically. */
  seed?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

/**
 * A banked-stone pile: quarried blocks (the Brain's described memories made
 * physical, Cave §4) heaped and waiting to be set. A faint tending-glow disc
 * underneath marks it as live material, not debris. One worker stands to set
 * from it.
 */
export function StonePile({
  position = [0, 0, 0],
  rotationY = 0,
  count = 9,
  seed = 1,
  areaId = 'build',
  idPrefix = 'pile',
}: StonePileProps) {
  const layout = useMemo(() => {
    const rocks: InstanceTransform[] = [];
    // Heap rocks in a loose cone: radius shrinks with height.
    for (let i = 0; i < count; i++) {
      const h = hash01(seed * 31 + i);
      const a = hash01(seed * 17 + i * 7) * Math.PI * 2;
      const layer = Math.floor(i / Math.max(1, Math.ceil(count / 3)));
      const r = (0.9 - layer * 0.28) * (0.5 + h * 0.5);
      const y = 0.25 + layer * 0.42;
      const s = 0.5 + hash01(seed * 53 + i) * 0.45;
      rocks.push({
        position: [Math.cos(a) * r, y, Math.sin(a) * r],
        rotation: [h * 3.1, a, hash01(i * 13) * 3.1],
        scale: [s, s * (0.7 + h * 0.4), s],
      });
    }
    return { rocks };
  }, [count, seed]);

  const anchors = useMemo<AgentAnchor[]>(
    () => [
      {
        id: `${idPrefix}.set.stand`,
        areaId,
        kind: 'stand' as const,
        ...placeAnchor([1.3, 0, 0], -Math.PI / 2, position, rotationY),
      },
    ],
    [areaId, idPrefix, position, rotationY]
  );
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      <InstancedProp name={`${idPrefix}.rock`} geometry={unitRock} material={stoneRough} transforms={layout.rocks} castShadow receiveShadow />
      {/* Tending-glow disc: live material marker */}
      <mesh rotation-x={-Math.PI / 2} position={[0, 0.012, 0]} scale={[2.0, 2.0, 1]} geometry={unitBox} material={lightTending} />
    </group>
  );
}

// ─── SurveyLine: glowing footprint of a future wing (Cave §4) ────────────────

interface SurveyLineProps {
  position?: [number, number, number];
  rotationY?: number;
  /** Footprint size in u. */
  size?: [number, number];
  seed?: number;
  idPrefix?: string;
}

/**
 * A survey-line footprint: glowing tending-register lines drawn on bare stone
 * where a future wing will rise, with tetra-capped corner stakes. "The roadmap
 * is geography" (Cave §4) — a survey line is a promise, rendered. No anchors
 * (nothing to engage); pure environmental intent.
 */
export function SurveyLine({
  position = [0, 0, 0],
  rotationY = 0,
  size = [6, 4],
  seed = 1,
  idPrefix = 'survey',
}: SurveyLineProps) {
  const layout = useMemo(() => {
    const lines: InstanceTransform[] = [];
    const stakes: InstanceTransform[] = [];
    const caps: InstanceTransform[] = [];
    const [w, d] = size;
    const t = 0.06;
    const y = 0.014;
    // Perimeter rectangle (4 thin glowing lines just above the floor).
    lines.push({ position: [0, y, -d / 2], scale: [w, 0.02, t] });
    lines.push({ position: [0, y, d / 2], scale: [w, 0.02, t] });
    lines.push({ position: [-w / 2, y, 0], scale: [t, 0.02, d] });
    lines.push({ position: [w / 2, y, 0], scale: [t, 0.02, d] });
    // A couple of deterministic interior division lines — hints of inner rooms.
    const splitX = -w / 2 + (0.3 + hash01(seed) * 0.4) * w;
    lines.push({ position: [splitX, y, 0], scale: [t, 0.02, d] });

    // Corner stakes with glowing tetra caps.
    for (const sx of [-w / 2, w / 2]) {
      for (const sz of [-d / 2, d / 2]) {
        stakes.push({ position: [sx, 0.2, sz], scale: [0.08, 0.4, 0.08] });
        caps.push({ position: [sx, 0.46, sz], scale: [0.22, 0.22, 0.22] });
      }
    }
    return { lines, stakes, caps };
  }, [size, seed]);

  return (
    <group position={position} rotation-y={rotationY}>
      <InstancedProp name={`${idPrefix}.line`} geometry={unitBox} material={lightTending} transforms={layout.lines} />
      <InstancedProp name={`${idPrefix}.stake`} geometry={unitCylinder8} material={metalGunmetal} transforms={layout.stakes} />
      <InstancedProp name={`${idPrefix}.cap`} geometry={unitTetra} material={lightTending} transforms={layout.caps} />
    </group>
  );
}
