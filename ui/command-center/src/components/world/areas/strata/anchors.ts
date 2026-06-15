// Strata scaffold mount points — the W1 → W2 interface.
// THE_CAVE_vision_bible.md §4 ("Scaffolding is promise") + §9: W2 builds the
// construction kit (scaffold lattice, crane, banked-stone piles, Cornerstone
// Gallery kit); W1 publishes WHERE that kit attaches in the vertical cave.
//
// This is deliberately separate from the FROZEN shared/anchors.ts: that module is
// the agent-seat registry (W2 props → W3 postures). THIS module is structural
// construction state — bare rock that future wings rise from. W1 owns and freezes
// it; W2 reads getMounts() and dresses each mount with the matching kit piece.
//
// Real scale: all positions world-space, 1 unit = 1 metre (WORLD_VIEW_BIBLE.md §1).

import type { StratumId } from './strata';

/**
 * What kind of construction the mount expects. Drives which kit piece W2 places
 * and the scaffold state it renders in (bible §4: "construction site at first
 * light" — five core chambers stand; survey lines glow where future wings rise).
 */
export type ScaffoldKind =
  /** Bare survey footprint on rock; nothing built yet — glowing outline only. */
  | 'survey'
  /** Scaffold lattice + work-lights stand here; stonework in progress. */
  | 'scaffold'
  /** Banked, quarried stone waiting to be set (bible §4: "builders bank material"). */
  | 'banked'
  /** Idle crane footprint — the gantry mounts here, reaching over the work. */
  | 'crane'
  /** Finished stonework — a wing that has already risen (crown-adjacent strata). */
  | 'built';

/**
 * One construction mount point in the cave. `progress` is the scaffold STATE
 * W1 ships it in for the blockout; W2/W3 later drive it from real banked daemon
 * events (bible §4 "construction rate is driven by real events"). At blockout
 * time it is static gray geometry.
 */
export interface ScaffoldMount {
  /** Stable id, e.g. 'carved.eastwing.survey'. */
  id: string;
  stratum: StratumId;
  kind: ScaffoldKind;
  /** World-space anchor: where the kit piece's origin sits. */
  position: [number, number, number];
  /** Y rotation the kit piece assumes (faces the cavern centre by default). */
  facing: number;
  /**
   * Footprint the kit fills, in metres [x, z]. Survey lines trace this rectangle;
   * scaffold lattice fills it; banked piles sit within it.
   */
  footprint: [number, number];
  /**
   * Blockout scaffold state 0..1 (0 = bare survey, 1 = built). Honesty bit for the
   * detail pass: W2 maps real banked-event volume onto this; never a usage bar.
   */
  progress: number;
}

// ── The mount table ─────────────────────────────────────────────────────────
// Footprints distribute around the cavern wall at each stratum, offset from the
// throat (centre [0,0,9]) so survey lines never cross the abyss. Maturity grades
// the default state: crown-adjacent strata are mostly built, the deep mining
// stratum is mostly bare survey on raw rock (bible §2 "depth is time").

function ring(
  stratum: StratumId,
  y: number,
  r: number,
  specs: Array<{ ang: number; kind: ScaffoldKind; progress: number; tag: string; fp?: [number, number] }>
): ScaffoldMount[] {
  return specs.map(({ ang, kind, progress, tag, fp }) => {
    const x = Math.cos(ang) * r;
    const z = 9 + Math.sin(ang) * r; // throat centre is at z=9
    return {
      id: `${stratum}.${tag}.${kind}`,
      stratum,
      kind,
      position: [x, y, z] as [number, number, number],
      facing: Math.atan2(9 - z, -x), // face the throat/cavern centre
      footprint: fp ?? [3, 3],
      progress,
    };
  });
}

export const SCAFFOLD_MOUNTS: ScaffoldMount[] = [
  // Verge (crown-adjacent, maturest): finished wings + one fresh scaffold.
  ...ring('verge', -4.5, 11, [
    { ang: 0, kind: 'built', progress: 1, tag: 'northwing', fp: [4, 3] },
    { ang: Math.PI * 0.66, kind: 'built', progress: 1, tag: 'eastwing', fp: [4, 3] },
    { ang: Math.PI * 1.33, kind: 'scaffold', progress: 0.7, tag: 'westwing', fp: [4, 3] },
  ]),
  // Carved Halls: a wing rising, one crane, banked stone staged.
  ...ring('carved', -10, 13, [
    { ang: Math.PI * 0.2, kind: 'scaffold', progress: 0.55, tag: 'gallery', fp: [5, 4] },
    { ang: Math.PI * 0.9, kind: 'crane', progress: 0.3, tag: 'gantry', fp: [3, 3] },
    { ang: Math.PI * 1.6, kind: 'banked', progress: 0.4, tag: 'staging', fp: [4, 4] },
  ]),
  // Rough Work: early scaffolds + survey footprints on bare rock.
  ...ring('roughwork', -16, 14, [
    { ang: Math.PI * 0.4, kind: 'scaffold', progress: 0.3, tag: 'spur', fp: [4, 3] },
    { ang: Math.PI, kind: 'survey', progress: 0.05, tag: 'plot-a', fp: [5, 4] },
    { ang: Math.PI * 1.5, kind: 'banked', progress: 0.2, tag: 'cairn', fp: [3, 3] },
  ]),
  // The Mining: where the work happens — mostly bare survey + the deepest crane.
  ...ring('mining', -22, 12, [
    { ang: Math.PI * 0.3, kind: 'survey', progress: 0.02, tag: 'plot-b', fp: [5, 5] },
    { ang: Math.PI * 1.1, kind: 'crane', progress: 0.1, tag: 'deepgantry', fp: [3, 3] },
    { ang: Math.PI * 1.8, kind: 'survey', progress: 0, tag: 'plot-c', fp: [4, 4] },
  ]),
];

const mountById = new Map(SCAFFOLD_MOUNTS.map((m) => [m.id, m]));

/** All scaffold mounts, optionally filtered to one stratum. */
export function getMounts(stratum?: StratumId): ScaffoldMount[] {
  return stratum ? SCAFFOLD_MOUNTS.filter((m) => m.stratum === stratum) : SCAFFOLD_MOUNTS;
}

export function getMount(id: string): ScaffoldMount | undefined {
  return mountById.get(id);
}

/** Mounts whose state is the given scaffold kind — convenience for W2 kit dressing. */
export function getMountsByKind(kind: ScaffoldKind): ScaffoldMount[] {
  return SCAFFOLD_MOUNTS.filter((m) => m.kind === kind);
}
