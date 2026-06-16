// W2 — TendingProps: what the tending agents (W3) hold and use (Cave §4).
// "The third agent state: tending. Between tasks, workers tend the world —
// hauling, setting stones, raising scaffold. Gray-warm, unhurried, honest."
// These are the props that state needs: a hauling sledge loaded with quarried
// stone, and a stone-setting station with the tools a worker sets a block from.
//
// HUD law (Cave §4, bible §8): tending is its OWN warm-gray register, NEVER
// amber. Every emissive here is lightTending — the load-glow on the sledge, the
// setting-station's mark line. No HUD amber, no cyan (cyan = finished carving).
//
// Anchors (the W2→W3 publish, frozen API): the sledge handle is a 'lean' (a
// worker leaning into the haul); the stone-setting station is a 'stand' (a
// worker bent to set a block) + a 'lean' at the tool board. Kinds frozen at
// seat/stand/lean — used creatively, none added.
//
// Instancing (bible §8): the sledge's stone load and the setting tools are
// InstancedProp.

import { useMemo } from 'react';
import type { AreaId, AgentAnchor } from '../shared/anchors';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox, unitCylinder8, unitRock, unitCone6, hash01 } from './geometries';
import {
  stoneRough,
  stoneDark,
  metalGunmetal,
  metalBronze,
  lightTending,
} from './materials';
import { placeAnchor, useRegisterAnchors } from './propUtils';

// ─── HaulingSledge: a loaded sledge a worker drags (Cave §4) ─────────────────

interface HaulingSledgeProps {
  position?: [number, number, number];
  rotationY?: number;
  /** Loaded with quarried stone, or empty (returning for the next load). */
  loaded?: boolean;
  seed?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

/**
 * A hauling sledge: a heavy timber-and-gunmetal skid with twin runners, a haul
 * handle a worker leans into, and (when loaded) a heap of quarried rock with a
 * faint tending-glow marking it as live material. The unhurried freight of the
 * tending state.
 */
export function HaulingSledge({
  position = [0, 0, 0],
  rotationY = 0,
  loaded = true,
  seed = 1,
  areaId = 'build',
  idPrefix = 'sledge',
}: HaulingSledgeProps) {
  const layout = useMemo(() => {
    const load: InstanceTransform[] = [];
    if (loaded) {
      // A small heap of quarried blocks on the deck.
      for (let i = 0; i < 5; i++) {
        const x = (hash01(seed * 7 + i) - 0.5) * 1.1;
        const z = (hash01(seed * 13 + i) - 0.5) * 0.7;
        const layer = i < 3 ? 0 : 1;
        const s = 0.45 + hash01(seed * 23 + i) * 0.3;
        load.push({
          position: [x, 0.45 + layer * 0.4, z],
          rotation: [hash01(i) * 3.1, hash01(i * 3) * 3.1, hash01(i * 5) * 3.1],
          scale: [s, s * 0.85, s],
        });
      }
    }
    return { load };
  }, [loaded, seed]);

  const anchors = useMemo<AgentAnchor[]>(
    () => [
      {
        id: `${idPrefix}.haul.lean`,
        areaId,
        kind: 'lean' as const,
        // At the front handle, facing the pull direction (+x local → forward).
        ...placeAnchor([1.6, 0, 0], -Math.PI / 2, position, rotationY),
      },
    ],
    [areaId, idPrefix, position, rotationY]
  );
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      {/* Twin runners (gunmetal skids) */}
      <mesh position={[0, 0.1, -0.45]} scale={[2.4, 0.16, 0.18]} geometry={unitBox} material={metalGunmetal} castShadow />
      <mesh position={[0, 0.1, 0.45]} scale={[2.4, 0.16, 0.18]} geometry={unitBox} material={metalGunmetal} castShadow />
      {/* Deck (timber-toned dark stone stand-in) */}
      <mesh position={[0, 0.22, 0]} scale={[2.2, 0.1, 1.1]} geometry={unitBox} material={stoneDark} castShadow receiveShadow />
      {/* Haul handle: a bent gunmetal bar at the front */}
      <mesh position={[1.25, 0.5, -0.4]} rotation-z={0.5} scale={[0.07, 0.9, 0.07]} geometry={unitCylinder8} material={metalGunmetal} />
      <mesh position={[1.25, 0.5, 0.4]} rotation-z={0.5} scale={[0.07, 0.9, 0.07]} geometry={unitCylinder8} material={metalGunmetal} />
      <mesh position={[1.55, 0.85, 0]} scale={[0.07, 0.07, 0.95]} geometry={unitCylinder8} material={metalBronze} />
      <InstancedProp name={`${idPrefix}.load`} geometry={unitRock} material={stoneRough} transforms={layout.load} castShadow receiveShadow />
      {loaded && (
        <mesh rotation-x={-Math.PI / 2} position={[0, 0.28, 0]} scale={[1.8, 1.0, 1]} geometry={unitBox} material={lightTending} />
      )}
    </group>
  );
}

// ─── StoneSettingStation: where a worker sets a block (Cave §4) ──────────────

interface StoneSettingStationProps {
  position?: [number, number, number];
  rotationY?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

/**
 * A stone-setting station: a block half-set into a course of masonry, a mortar
 * trough, and a leaning board of setting tools (mallet, level bar, plumb). A
 * faint tending-glow setting-line marks where the next stone lands. The worker
 * bends to set (stand anchor) or leans to take a tool (lean anchor).
 */
export function StoneSettingStation({
  position = [0, 0, 0],
  rotationY = 0,
  areaId = 'build',
  idPrefix = 'setting',
}: StoneSettingStationProps) {
  const layout = useMemo(() => {
    const course: InstanceTransform[] = [];
    const tools: InstanceTransform[] = [];
    const toolHeads: InstanceTransform[] = [];
    // A short course of set blocks, with one block half-set (raised, off-grid).
    for (let i = 0; i < 4; i++) {
      const x = -1.35 + i * 0.9;
      course.push({ position: [x, 0.25, 0], scale: [0.85, 0.5, 0.7] });
    }
    // The block being set: lifted and slightly skewed above the next slot.
    course.push({ position: [1.4, 0.7, 0], rotation: [0, 0.08, 0.05], scale: [0.85, 0.5, 0.7] });

    // Setting tools leaning on the board.
    const slots = [-0.4, 0, 0.4];
    for (let i = 0; i < slots.length; i++) {
      const x = slots[i];
      tools.push({ position: [x, 0.7, -1.1], rotation: [0.2, 0, 0], scale: [0.045, 1.2, 0.045] });
      toolHeads.push({ position: [x, 1.28, -1.05], rotation: [Math.PI, 0, i * 0.4], scale: [0.16, 0.22, 0.1] });
    }
    return { course, tools, toolHeads };
  }, []);

  const anchors = useMemo<AgentAnchor[]>(
    () => [
      {
        id: `${idPrefix}.set.stand`,
        areaId,
        kind: 'stand' as const,
        // Bent at the half-set block, facing the course.
        ...placeAnchor([1.4, 0, 0.9], Math.PI, position, rotationY),
      },
      {
        id: `${idPrefix}.tools.lean`,
        areaId,
        kind: 'lean' as const,
        ...placeAnchor([0, 0, -1.7], 0, position, rotationY),
      },
    ],
    [areaId, idPrefix, position, rotationY]
  );
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      <InstancedProp name={`${idPrefix}.course`} geometry={unitBox} material={stoneRough} transforms={layout.course} castShadow receiveShadow />
      {/* Mortar trough (dark stone) */}
      <mesh position={[-1.8, 0.2, 0.6]} scale={[0.5, 0.4, 0.8]} geometry={unitBox} material={stoneDark} castShadow />
      {/* Tool board */}
      <mesh position={[0, 0.7, -1.2]} rotation-x={0.2} scale={[1.4, 1.4, 0.08]} geometry={unitBox} material={stoneDark} castShadow />
      <InstancedProp name={`${idPrefix}.tool`} geometry={unitCylinder8} material={metalGunmetal} transforms={layout.tools} castShadow />
      <InstancedProp name={`${idPrefix}.toolHead`} geometry={unitCone6} material={metalBronze} transforms={layout.toolHeads} castShadow />
      {/* Tending setting-line: where the next stone lands. */}
      <mesh rotation-x={-Math.PI / 2} position={[1.4, 0.015, 0]} scale={[0.85, 0.7, 1]} geometry={unitBox} material={lightTending} />
    </group>
  );
}
