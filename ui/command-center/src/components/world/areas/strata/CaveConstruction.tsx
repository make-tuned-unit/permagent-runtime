// CaveConstruction — the W1 → W2 reconnect. THE_CAVE_vision_bible.md §4.
// W1 publishes WHERE the kit attaches (./anchors, getMountsByKind); W2's real
// construction kit (Scaffold lattice, IdleCrane, StonePile, SurveyLine) dresses
// each mount. This REPLACES the gray blockout stand-ins (ScaffoldMounts.tsx /
// SurveyLines.tsx) for the kinds W2 owns:
//
//   ScaffoldMount.kind  →  W2 prop
//   ──────────────────     ─────────────────────────────────────────────
//   'scaffold'          →  <Scaffold>      (light-lattice + amber work-lamps)
//   'crane'             →  <IdleCrane>     (idle bronze gantry at first light)
//   'banked'            →  <StonePile>     (banked quarried stone, waiting)
//   'survey'            →  <SurveyLine>    (glowing footprint of a future wing)
//   'built'             →  (left to W1's BuiltWings blockout — no W2 finished
//                           -wing prop exists yet; out of this step's scope)
//
// Each kit piece already takes (position, rotationY); this is pure wiring off the
// real ScaffoldMount table — no per-mount literals live here.
//
// ── W2 → W3 seam (construction stand anchors) ────────────────────────────────
// For every 'scaffold' and 'banked' mount we register one 'stand'-kind AgentAnchor
// where a worker stands to build, under the STABLE id `${mount.id}.build`. W3
// consumes these ids verbatim. They register on mount and unregister with the
// area (lazy-descent chunk) via useRegisterAnchors. The kit's own per-prop seat/
// lean anchors (scaffold rails, pile-set stand) are unaffected — these are the
// canonical, W1-mount-derived build positions W3 was promised.

import { useMemo } from 'react';
import { Scaffold, IdleCrane, StonePile, SurveyLine } from '../../props';
import type { AgentAnchor } from '../../shared/anchors';
import { useRegisterAnchors } from '../../props';
import {
  getMountsByKind,
  type ScaffoldMount,
} from './anchors';

// The cave/strata cavern is the 'build' area in the frozen anchor registry.
const AREA = 'build' as const;

/** Scaffold lattice sizing from a mount footprint + its blockout progress. */
function scaffoldDims(m: ScaffoldMount): { bays: number; levels: number } {
  const [fx] = m.footprint;
  // ~2u per bay; at least 2 bays so the lattice reads as a lattice.
  const bays = Math.max(2, Math.round(fx / 2));
  // Taller as the wing rises (progress 0..1 → 2..4 levels).
  const levels = Math.max(2, Math.round(2 + m.progress * 2));
  return { bays, levels };
}

/** Banked-pile density from footprint + progress (a busier bank as it fills). */
function pileCount(m: ScaffoldMount): number {
  const [fx, fz] = m.footprint;
  const area = fx * fz;
  return Math.round(6 + area * 0.3 + m.progress * 6);
}

export function CaveConstruction() {
  const scaffolds = getMountsByKind('scaffold');
  const cranes = getMountsByKind('crane');
  const banked = getMountsByKind('banked');
  const surveys = getMountsByKind('survey');

  // W2 → W3: one 'stand' build-anchor per scaffold and banked mount, at the
  // mount origin facing the cavern centre (mount.facing). Stable id `<id>.build`.
  const buildAnchors = useMemo<AgentAnchor[]>(
    () =>
      [...scaffolds, ...banked].map((m) => ({
        id: `${m.id}.build`,
        areaId: AREA,
        kind: 'stand' as const,
        position: m.position,
        facing: m.facing,
      })),
    [scaffolds, banked]
  );
  useRegisterAnchors(buildAnchors);

  return (
    <group>
      {scaffolds.map((m) => {
        const { bays, levels } = scaffoldDims(m);
        return (
          <Scaffold
            key={m.id}
            position={m.position}
            rotationY={m.facing}
            bays={bays}
            levels={levels}
            areaId={AREA}
            idPrefix={m.id}
          />
        );
      })}
      {cranes.map((m) => (
        <IdleCrane key={m.id} position={m.position} rotationY={m.facing} />
      ))}
      {banked.map((m) => (
        <StonePile
          key={m.id}
          position={m.position}
          rotationY={m.facing}
          count={pileCount(m)}
          seed={Math.round(m.position[0] * 7.1 + m.position[2] * 3.3 + 11)}
          areaId={AREA}
          idPrefix={m.id}
        />
      ))}
      {surveys.map((m) => (
        <SurveyLine
          key={m.id}
          position={m.position}
          rotationY={m.facing}
          size={m.footprint}
          seed={Math.round(m.position[0] * 5.7 + m.position[2] * 2.9 + 7)}
          idPrefix={m.id}
        />
      ))}
    </group>
  );
}
