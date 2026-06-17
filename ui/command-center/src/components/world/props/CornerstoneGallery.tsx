// W2 — CornerstoneGallery: the founding-room template kit (Cave §6).
// The five template elements as composable props. This delivers FORM only —
// population from real project data (completed-goal reliefs, entity statuary,
// the project's hardest moment for the Cracked Stone) is future wiring. No fake
// content: plinths and relief panels render as blank template surfaces, the
// places where real data will mount. The ONLY inscribed text is the founder's
// cornerstone mark, which is embedded in the template by design (Cave §6.4).
//
// The five elements (Cave §6):
//   1. SwitchbackAscent — you walk UP through your first project, not into a room.
//   2. GoalRelief panels — completed goals mounted as wall reliefs (blank template).
//   3. StatuaryPlinth — key entities as statuary plinths (blank template).
//   4. CrackedStone — one deliberately imperfect stone, BRACED not replaced.
//   5. CornerstoneInscription — tiny, on the first stone: "It starts in the dark."
//   + LandingTerrace — the top terrace looking back down and up toward the Mouth.
//
// Material grammar (Cave §3): ascent and terrace are composed stone (the wing is
// maturing work); relief/plinth frames are bronze; the bracing on the Cracked
// Stone is honest gunmetal strapping (you build ON your history, §6.3); carved
// accents are cyan (the agents' understanding). The inscription is engraved
// cyan, tiny — it reads only up close, as a founder's mark should.
//
// Instancing (bible §8): switchback steps, relief panels and plinths are all
// InstancedProp. The inscription + cracked stone are unique focal singletons.
// Anchors: the LandingTerrace publishes 1 'stand' (the user/agent looking back);
// StatuaryPlinth publishes 1 'stand' per call (an agent presenting at a plinth).

import { useMemo } from 'react';
import { Text } from '@react-three/drei';
import type { AreaId, AgentAnchor } from '../shared/anchors';
import { ENV } from '../shared/palette';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, unitCylinder16 } from './geometries';
import {
  stoneMarble,
  stoneDark,
  stoneRough,
  metalGunmetal,
  metalBronze,
  lightCyan,
} from './materials';
import { placeAnchor, useRegisterAnchors } from './propUtils';

// ─── SwitchbackAscent: you walk UP through your first project (Cave §6.1) ─────

interface SwitchbackAscentProps {
  position?: [number, number, number];
  rotationY?: number;
  /** Number of switchback flights (each reverses direction). */
  flights?: number;
  idPrefix?: string;
}

const STEPS_PER_FLIGHT = 6;
const STEP_RISE = 0.4;
const STEP_RUN = 0.9;
const FLIGHT_W = 3.0;

/**
 * The switchback ascent: stone flights that reverse direction at each landing,
 * climbing the gallery. "Because the founding project was a climb and so is
 * everyone's" (Cave §6.1). Returns its own top position via TOP_Y for callers
 * placing the landing terrace.
 */
export function SwitchbackAscent({
  position = [0, 0, 0],
  rotationY = 0,
  flights = 3,
  idPrefix = 'ascent',
}: SwitchbackAscentProps) {
  const layout = useMemo(() => {
    const steps: InstanceTransform[] = [];
    const landings: InstanceTransform[] = [];
    const rails: InstanceTransform[] = [];

    let y = 0;
    let z = 0;
    for (let f = 0; f < flights; f++) {
      const dir = f % 2 === 0 ? 1 : -1; // alternate flight direction along x
      const xStart = dir > 0 ? -FLIGHT_W / 2 : FLIGHT_W / 2;
      for (let s = 0; s < STEPS_PER_FLIGHT; s++) {
        const x = xStart + dir * (s + 0.5) * STEP_RUN;
        y += STEP_RISE;
        steps.push({ position: [x, y - STEP_RISE / 2, z], scale: [STEP_RUN, STEP_RISE, FLIGHT_W] });
      }
      // Landing slab at the turn.
      z += FLIGHT_W;
      landings.push({ position: [0, y, z - FLIGHT_W / 2], scale: [FLIGHT_W + STEP_RUN, 0.18, FLIGHT_W] });
      // A short carved cyan rail seam along the outer edge of the landing.
      rails.push({ position: [(FLIGHT_W / 2 + 0.4) * dir, y + 0.5, z - FLIGHT_W / 2], scale: [0.06, 1.0, FLIGHT_W * 0.8] });
    }
    return { steps, landings, rails, topY: y, topZ: z };
  }, [flights]);

  return (
    <group position={position} rotation-y={rotationY}>
      <InstancedProp name={`${idPrefix}.step`} geometry={unitBox} material={stoneMarble} transforms={layout.steps} castShadow receiveShadow />
      <InstancedProp name={`${idPrefix}.landing`} geometry={unitBox} material={stoneDark} transforms={layout.landings} castShadow receiveShadow />
      <InstancedProp name={`${idPrefix}.rail`} geometry={unitBox} material={lightCyan} transforms={layout.rails} />
    </group>
  );
}

/** Top-of-ascent offset for a given flight count — callers place the terrace. */
export function ascentTop(flights = 3): { y: number; z: number } {
  return { y: flights * STEPS_PER_FLIGHT * STEP_RISE, z: flights * FLIGHT_W };
}

// ─── GoalReliefWall: completed goals as wall reliefs (Cave §6.2, blank) ──────

interface GoalReliefWallProps {
  position?: [number, number, number];
  rotationY?: number;
  /** How many relief slots the wall offers (template capacity). */
  slots?: number;
  idPrefix?: string;
}

/**
 * A relief wall: a stone wall with recessed bronze-framed panels, each a blank
 * template slot where a completed goal's relief will mount (Cave §6.2). Renders
 * empty by design — no fake goals.
 */
export function GoalReliefWall({
  position = [0, 0, 0],
  rotationY = 0,
  slots = 4,
  idPrefix = 'relief',
}: GoalReliefWallProps) {
  const layout = useMemo(() => {
    const frames: InstanceTransform[] = [];
    const panels: InstanceTransform[] = [];
    const spacing = 1.8;
    const x0 = (-(slots - 1) * spacing) / 2;
    for (let i = 0; i < slots; i++) {
      const x = x0 + i * spacing;
      frames.push({ position: [x, 1.8, 0.04], scale: [1.5, 2.0, 0.12] });
      panels.push({ position: [x, 1.8, 0.11], scale: [1.3, 1.8, 0.04] });
    }
    return { frames, panels };
  }, [slots]);

  const wallW = slots * 1.8 + 1.0;
  return (
    <group position={position} rotation-y={rotationY}>
      {/* The wall itself */}
      <mesh position={[0, 1.8, 0]} scale={[wallW, 3.6, 0.3]} geometry={unitBox} material={stoneMarble} castShadow receiveShadow />
      <InstancedProp name={`${idPrefix}.frame`} geometry={unitBox} material={metalBronze} transforms={layout.frames} castShadow />
      {/* Blank recessed template panels (dark stone — content mounts here). */}
      <InstancedProp name={`${idPrefix}.panel`} geometry={unitBox} material={stoneDark} transforms={layout.panels} />
    </group>
  );
}

// ─── StatuaryPlinth: a key entity as statuary (Cave §6.2, blank) ─────────────

interface StatuaryPlinthProps {
  position?: [number, number, number];
  rotationY?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

/**
 * A statuary plinth: a stepped marble plinth with a bronze cap and a faint cyan
 * top-seam, where a project's key entity will stand as statuary (Cave §6.2 —
 * the founder's gallery holds Henry, the Librarian, Spectral). Renders WITHOUT a
 * figure: the figure is real entity data, future wiring. An agent stands beside
 * it to present.
 */
export function StatuaryPlinth({
  position = [0, 0, 0],
  rotationY = 0,
  areaId = 'build',
  idPrefix = 'plinth',
}: StatuaryPlinthProps) {
  const anchors = useMemo<AgentAnchor[]>(
    () => [
      {
        id: `${idPrefix}.present.stand`,
        areaId,
        kind: 'stand' as const,
        ...placeAnchor([1.2, 0, 0], -Math.PI / 2, position, rotationY),
      },
    ],
    [areaId, idPrefix, position, rotationY]
  );
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      <mesh position={[0, 0.15, 0]} scale={[1.4, 0.3, 1.4]} geometry={unitBox} material={stoneDark} castShadow receiveShadow />
      <mesh position={[0, 0.7, 0]} scale={[1.0, 1.0, 1.0]} geometry={unitCylinder16} material={stoneMarble} castShadow />
      <mesh position={[0, 1.24, 0]} scale={[1.1, 0.12, 1.1]} geometry={unitCylinder16} material={metalBronze} castShadow />
      {/* Faint cyan top seam: the slot where statuary stands. */}
      <mesh rotation-x={-Math.PI / 2} position={[0, 1.31, 0]} scale={[0.8, 0.8, 1]} geometry={unitCylinder16} material={lightCyan} />
    </group>
  );
}

// ─── CrackedStone: one imperfect stone, BRACED not replaced (Cave §6.3) ──────

interface CrackedStoneProps {
  position?: [number, number, number];
  rotationY?: number;
  idPrefix?: string;
}

/**
 * The Cracked Stone (Cave §6.3): every gallery preserves one deliberately
 * imperfect stone — a split block, braced with honest gunmetal strapping rather
 * than replaced. "You don't renovate your history; you build on it." The crack
 * is a visible dark gap; the braces are real straps, not decoration. Populated
 * honestly from the project's hardest moment is future wiring; the FORM is the
 * braced split itself.
 */
export function CrackedStone({
  position = [0, 0, 0],
  rotationY = 0,
  idPrefix = 'cracked',
}: CrackedStoneProps) {
  const layout = useMemo(() => {
    const straps: InstanceTransform[] = [];
    // Three gunmetal straps wrapping the block across the crack line.
    for (const y of [0.5, 1.0, 1.5]) {
      straps.push({ position: [0, y, 0], scale: [1.7, 0.14, 1.7] });
    }
    return { straps };
  }, []);

  return (
    <group position={position} rotation-y={rotationY}>
      {/* Two halves of a split block, a dark gap between them (the crack). */}
      <mesh position={[-0.42, 1.0, 0]} rotation-z={0.04} scale={[0.78, 2.0, 1.5]} geometry={unitBox} material={stoneRough} castShadow receiveShadow />
      <mesh position={[0.42, 1.0, 0]} rotation-z={-0.03} scale={[0.78, 2.0, 1.5]} geometry={unitBox} material={stoneRough} castShadow receiveShadow />
      {/* Honest bracing straps across the crack. */}
      <InstancedProp name={`${idPrefix}.strap`} geometry={unitBox} material={metalGunmetal} transforms={layout.straps} castShadow />
      {/* Vertical tension rods at the strap ends. */}
      <mesh position={[-0.85, 1.0, 0]} scale={[0.1, 1.6, 0.1]} geometry={unitBox} material={metalGunmetal} />
      <mesh position={[0.85, 1.0, 0]} scale={[0.1, 1.6, 0.1]} geometry={unitBox} material={metalGunmetal} />
    </group>
  );
}

// ─── CornerstoneInscription: the founder's mark (Cave §6.4) ──────────────────

interface CornerstoneInscriptionProps {
  position?: [number, number, number];
  rotationY?: number;
}

/** The founder's inscription text — embedded in the template by design. */
export const CORNERSTONE_TEXT = 'It starts in the dark.';

/**
 * The cornerstone inscription (Cave §6.4): a small rough first stone with the
 * founder's mark engraved tiny, glowing faint cyan. The one sanctioned text in
 * the gallery — it is the template's own founding mark, not project content.
 * Reads only up close, as a cornerstone should.
 */
export function CornerstoneInscription({
  position = [0, 0, 0],
  rotationY = 0,
}: CornerstoneInscriptionProps) {
  return (
    <group position={position} rotation-y={rotationY}>
      {/* The first stone */}
      <mesh position={[0, 0.35, 0]} scale={[1.6, 0.7, 0.7]} geometry={unitBox} material={stoneRough} castShadow receiveShadow />
      {/* Engraved, tiny, faint cyan — the founder's mark. */}
      <Text
        position={[0, 0.4, 0.37]}
        fontSize={0.085}
        color={ENV.neonCyan}
        anchorX="center"
        anchorY="middle"
        maxWidth={1.4}
        textAlign="center"
        outlineWidth={0}
      >
        {CORNERSTONE_TEXT}
      </Text>
    </group>
  );
}

// ─── LandingTerrace: the view back down and up (Cave §6.5) ────────────────────

interface LandingTerraceProps {
  position?: [number, number, number];
  rotationY?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

/**
 * The landing terrace (Cave §6.5): a small stone terrace at the top of the
 * gallery with a low parapet, where the user sees how far they've come — a view
 * back down the gallery and up toward the Mouth. A carved cyan sight-seam in the
 * floor leads the eye up. One stand anchor at the parapet (the looking-back
 * posture for W3).
 */
export function LandingTerrace({
  position = [0, 0, 0],
  rotationY = 0,
  areaId = 'build',
  idPrefix = 'terrace',
}: LandingTerraceProps) {
  const layout = useMemo(() => {
    const parapets: InstanceTransform[] = [];
    // Low parapet around three sides (open toward the ascent it caps).
    parapets.push({ position: [0, 0.5, -2.4], scale: [5.0, 0.6, 0.3] });
    parapets.push({ position: [-2.4, 0.5, 0], scale: [0.3, 0.6, 5.0] });
    parapets.push({ position: [2.4, 0.5, 0], scale: [0.3, 0.6, 5.0] });
    // A few deterministic merlon caps (bronze) so the parapet reads as built.
    const merlons: InstanceTransform[] = [];
    for (let i = 0; i < 5; i++) {
      const x = -2.0 + i * 1.0;
      merlons.push({ position: [x, 0.85, -2.4], scale: [0.4, 0.2, 0.34] });
    }
    return { parapets, merlons };
  }, []);

  const anchors = useMemo<AgentAnchor[]>(
    () => [
      {
        id: `${idPrefix}.lookback.stand`,
        areaId,
        kind: 'stand' as const,
        // Stand at the open edge, facing back down the gallery (toward -z origin).
        ...placeAnchor([0, 0, 1.6], 0, position, rotationY),
      },
    ],
    [areaId, idPrefix, position, rotationY]
  );
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      {/* Terrace floor */}
      <mesh position={[0, 0.1, 0]} scale={[5.0, 0.2, 5.0]} geometry={unitBox} material={stoneMarble} castShadow receiveShadow />
      <InstancedProp name={`${idPrefix}.parapet`} geometry={unitBox} material={stoneDark} transforms={layout.parapets} castShadow receiveShadow />
      <InstancedProp name={`${idPrefix}.merlon`} geometry={unitBox} material={metalBronze} transforms={layout.merlons} castShadow />
      {/* Carved cyan sight-seam pointing up toward the Mouth. */}
      <mesh position={[0, 0.21, 1.8]} scale={[0.08, 0.02, 2.4]} geometry={unitBox} material={lightCyan} />
    </group>
  );
}
