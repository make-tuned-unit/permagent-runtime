// W2 — StratumProps: per-stratum prop families (Cave §3, §9 W2).
// "The gradient between the strata is the biography." These families express
// maturity by where they sit on that gradient — the primal living-cave stratum
// (low) vs the composed forum (high), with intermediate mixes in between.
//
// PRIMAL (living cave, Cave §3 low): crude tools left honest, the Librarian's
// amber lamp moving in the dark, mineral glitter and root-mass in cracks, rough
// rock. Tool marks left honest; nature feral and co-authoring.
//
// Material grammar (Cave §3, BINDING):
//   • stone = the given world  → stoneRough / stoneDark
//   • water + growing things = THE USER → props NEVER cut living things, never
//     dam water. RootMass routes AROUND a crack; it frames, it does not block.
//     (There is no "cut root" / "dammed spring" prop in this module by design.)
//   • light-veins + carved work = the AGENTS' understanding → lightCyan, used
//     sparingly and only where the cut reads as deliberate carving.
//
// The composed (forum-high) furniture is mostly delivered by round-1 families
// (WorkstationCluster, AmbientProps, BrainShelves, etc.). This module fills the
// PRIMAL end and the intermediate-maturity props that round 1 lacked.
//
// Instancing (bible §8): tool racks, mineral scatter, and root strands are all
// InstancedProp. The Librarian's lamp is a singleton focal prop (one per scene).
// Anchors: a CrudeToolRack publishes 1 'lean'; the LibrariansLamp none (it is a
// carried/placed light, W3 owns who holds it).

import { useMemo, useRef } from 'react';
import * as THREE from 'three';
import { useFrame } from '@react-three/fiber';
import { getReduceMotion } from '../../../styles/tokens';
import type { AreaId, AgentAnchor } from '../shared/anchors';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, unitCylinder8, unitCone6, unitTetra, unitRock, hash01 } from './geometries';
import {
  stoneRough,
  stoneDark,
  metalGunmetal,
  metalBronze,
  lightAmberWork,
  lightCyan,
} from './materials';
import { placeAnchor, useRegisterAnchors } from './propUtils';

// ─── CrudeToolRack: the agents' oldest, clumsiest work, preserved ────────────

interface CrudeToolRackProps {
  position?: [number, number, number];
  rotationY?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

/**
 * A crude tool rack: a rough stone trestle leaning crude picks, mallets and
 * chisels (cone heads on cylinder hafts). The primal stratum's working kit —
 * clumsy, honest, never renovated (Cave §3 "the agents' oldest, clumsiest work
 * survives unpolished"). One worker leans here to take a tool.
 */
export function CrudeToolRack({
  position = [0, 0, 0],
  rotationY = 0,
  areaId = 'build',
  idPrefix = 'toolrack',
}: CrudeToolRackProps) {
  const layout = useMemo(() => {
    const hafts: InstanceTransform[] = [];
    const heads: InstanceTransform[] = [];
    // Five tools leaning against the trestle bar at slightly varied angles.
    const slots = [-0.7, -0.35, 0, 0.35, 0.7];
    for (let i = 0; i < slots.length; i++) {
      const x = slots[i];
      const lean = (hash01(i * 19) - 0.5) * 0.18;
      const len = 1.3 + hash01(i * 7) * 0.3;
      hafts.push({
        position: [x, len / 2 + 0.05, 0],
        rotation: [lean, 0, lean * 1.5],
        scale: [0.05, len, 0.05],
      });
      // Head: a wedge (pick/mallet) at the top, oriented by tool index.
      heads.push({
        position: [x, len + 0.05, 0],
        rotation: [Math.PI, 0, i % 2 === 0 ? 0.6 : -0.4],
        scale: [0.22, 0.3, 0.12],
      });
    }
    return { hafts, heads };
  }, []);

  const anchors = useMemo<AgentAnchor[]>(
    () => [
      {
        id: `${idPrefix}.take.lean`,
        areaId,
        kind: 'lean' as const,
        ...placeAnchor([0, 0, 0.9], Math.PI, position, rotationY),
      },
    ],
    [areaId, idPrefix, position, rotationY]
  );
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      {/* Rough stone trestle: two leg blocks + a crossbar */}
      <mesh position={[-0.85, 0.4, 0]} scale={[0.18, 0.8, 0.4]} geometry={unitBox} material={stoneRough} castShadow />
      <mesh position={[0.85, 0.4, 0]} scale={[0.18, 0.8, 0.4]} geometry={unitBox} material={stoneRough} castShadow />
      <mesh position={[0, 0.78, 0]} scale={[1.9, 0.12, 0.18]} geometry={unitBox} material={stoneDark} castShadow />
      <InstancedProp name={`${idPrefix}.haft`} geometry={unitCylinder8} material={metalGunmetal} transforms={layout.hafts} castShadow />
      <InstancedProp name={`${idPrefix}.head`} geometry={unitCone6} material={metalBronze} transforms={layout.heads} castShadow />
    </group>
  );
}

// ─── LibrariansLamp: the amber lamp moving in the dark (Cave §3) ──────────────

interface LibrariansLampProps {
  position?: [number, number, number];
  rotationY?: number;
  /** Faint flicker of the flame disc (the lamp "moving in the dark"). */
  animated?: boolean;
}

/**
 * The Librarian's lamp: a worn bronze stem, a cone shade, and an amber flame
 * disc that flickers faintly. The defining warm point of the primal dark
 * (Cave §3). A WORK-light (lightAmberWork) — warm, not HUD amber. A focal
 * singleton: place one where the Librarian mines.
 */
export function LibrariansLamp({
  position = [0, 0, 0],
  rotationY = 0,
  animated = true,
}: LibrariansLampProps) {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  const flameRef = useRef<THREE.Mesh>(null);

  useFrame(({ clock }) => {
    if (!animated || reduceMotion || !flameRef.current) return;
    // Lamp-flame flicker: small irregular scale wobble, no color/intensity churn
    // on the shared material (would leak into other lightAmberWork props).
    const t = clock.elapsedTime;
    const flick = 1 + (Math.sin(t * 11.3) * 0.5 + Math.sin(t * 7.1) * 0.5) * 0.06;
    flameRef.current.scale.set(0.16 * flick, 0.22 * flick, 0.16 * flick);
  });

  return (
    <group position={position} rotation-y={rotationY}>
      {/* Base + stem (worn bronze) */}
      <mesh position={[0, 0.05, 0]} scale={[0.3, 0.1, 0.3]} geometry={unitCylinder8} material={metalBronze} castShadow />
      <mesh position={[0, 0.5, 0]} scale={[0.07, 0.9, 0.07]} geometry={unitCylinder8} material={metalBronze} />
      {/* Shade (cone, opening down) */}
      <mesh position={[0, 1.0, 0]} rotation-x={Math.PI} scale={[0.34, 0.3, 0.34]} geometry={unitCone6} material={metalBronze} castShadow />
      {/* Flame disc */}
      <mesh ref={flameRef} position={[0, 0.92, 0]} scale={[0.16, 0.22, 0.16]} geometry={unitTetra} material={lightAmberWork} />
    </group>
  );
}

// ─── MineralVein: mineral glitter + a carved cut (Cave §3) ────────────────────

interface MineralVeinProps {
  position?: [number, number, number];
  rotationY?: number;
  seed?: number;
  /** Whether a carved cyan cut runs through the rock (agents' understanding). */
  carved?: boolean;
  idPrefix?: string;
}

/**
 * A mineral vein in raw rock: a rough boulder mass studded with tetra mineral
 * glitter, optionally crossed by a single carved cyan channel (Cave §3
 * "light-veins/carved work = the agents' understanding"). The carved cut is the
 * only place the primal stratum earns cyan — used sparingly.
 */
export function MineralVein({
  position = [0, 0, 0],
  rotationY = 0,
  seed = 1,
  carved = true,
  idPrefix = 'mineral',
}: MineralVeinProps) {
  const layout = useMemo(() => {
    const masses: InstanceTransform[] = [];
    const glints: InstanceTransform[] = [];
    // A small cluster of rough boulders.
    for (let i = 0; i < 3; i++) {
      const a = hash01(seed * 11 + i) * Math.PI * 2;
      const r = hash01(seed * 23 + i) * 0.6;
      const s = 0.8 + hash01(seed * 41 + i) * 0.8;
      masses.push({
        position: [Math.cos(a) * r, s * 0.45, Math.sin(a) * r],
        rotation: [hash01(i) * 3.1, a, hash01(i * 5) * 3.1],
        scale: [s, s * 0.9, s],
      });
    }
    // Mineral glitter scattered across the cluster face.
    for (let i = 0; i < 7; i++) {
      const a = hash01(seed * 13 + i * 3) * Math.PI * 2;
      const r = 0.3 + hash01(seed * 29 + i) * 0.7;
      glints.push({
        position: [Math.cos(a) * r, 0.4 + hash01(seed * 31 + i) * 0.9, Math.sin(a) * r],
        rotation: [hash01(i) * 3.1, hash01(i * 2) * 3.1, hash01(i * 3) * 3.1],
        scale: [0.09, 0.09, 0.09],
      });
    }
    return { masses, glints };
  }, [seed]);

  return (
    <group position={position} rotation-y={rotationY}>
      <InstancedProp name={`${idPrefix}.mass`} geometry={unitRock} material={stoneRough} transforms={layout.masses} castShadow receiveShadow />
      <InstancedProp name={`${idPrefix}.glint`} geometry={unitTetra} material={lightCyan} transforms={layout.glints} />
      {carved && (
        // A single carved channel through the rock: the agents' understanding,
        // routed not imposed (Cave §3 "a second current learning the river's path").
        <mesh position={[0, 0.9, 0.05]} rotation-z={0.5} scale={[0.05, 1.6, 0.05]} geometry={unitBox} material={lightCyan} />
      )}
    </group>
  );
}

// ─── RootMass: growing things the agents build AROUND, never cut (Cave §3) ───

interface RootMassProps {
  position?: [number, number, number];
  rotationY?: number;
  seed?: number;
  idPrefix?: string;
}

/**
 * Root-mass in a crack (Cave §3): the user's growing things, rendered as a knot
 * of bronze-toned root strands the architecture FRAMES rather than cuts. There
 * is deliberately no severed-root variant — the material law forbids cutting
 * living things. A worker building here routes around it.
 *
 * (Water/spring is W4's domain — atmosphere; W2 only supplies the framing stone
 * lip a worker would set AROUND a spring, never a plug that dams it.)
 */
export function RootMass({
  position = [0, 0, 0],
  rotationY = 0,
  seed = 1,
  idPrefix = 'root',
}: RootMassProps) {
  const layout = useMemo(() => {
    const strands: InstanceTransform[] = [];
    const framingStones: InstanceTransform[] = [];
    // A spray of root strands fanning up out of a crack — no two cut, all curve
    // outward (expressed via varied lean, not severance).
    for (let i = 0; i < 6; i++) {
      const a = (i / 6) * Math.PI * 2;
      const lean = 0.3 + hash01(seed * 7 + i) * 0.4;
      const len = 0.7 + hash01(seed * 17 + i) * 0.6;
      strands.push({
        position: [Math.cos(a) * 0.15, len / 2, Math.sin(a) * 0.15],
        rotation: [Math.cos(a) * lean, a, Math.sin(a) * lean],
        scale: [0.04, len, 0.04],
      });
    }
    // Framing stones the agents SET AROUND the roots — building around, not over.
    for (let i = 0; i < 4; i++) {
      const a = (i / 4) * Math.PI * 2 + Math.PI / 4;
      framingStones.push({
        position: [Math.cos(a) * 0.7, 0.12, Math.sin(a) * 0.7],
        rotation: [0, a, 0],
        scale: [0.4, 0.24, 0.3],
      });
    }
    return { strands, framingStones };
  }, [seed]);

  return (
    <group position={position} rotation-y={rotationY}>
      <InstancedProp name={`${idPrefix}.strand`} geometry={unitCylinder8} material={metalBronze} transforms={layout.strands} castShadow />
      <InstancedProp name={`${idPrefix}.frame`} geometry={unitBox} material={stoneDark} transforms={layout.framingStones} castShadow receiveShadow />
    </group>
  );
}
