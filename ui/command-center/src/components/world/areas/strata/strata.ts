// Strata registry — THE_CAVE_vision_bible.md §2 ("Depth is time") + §9 W1 scope.
// The vertical cave system beneath the crown. The crown (rotunda at y=0, dome to
// y≈18) is the finished forum nearest the light; descending in -y walks BACKWARD
// through the agents' maturation — top strata maturer, deepest strata rawest rock.
//
// W1-owned. Real scale: 1 unit = 1 metre (WORLD_VIEW_BIBLE.md §1). This module is
// pure data — the depth bands consumed by the blockout geometry, the throat, the
// survey-line footprints, and the scaffold mount-point registry (./anchors).

/** Strata IDs, crown-adjacent (maturest) to deepest (rawest). */
export type StratumId = 'verge' | 'carved' | 'roughwork' | 'mining' | 'bedrock';

/**
 * Maturity grades the material gradient (bible §3, "two strata one gradient").
 * 1 = forum-clean (crown-adjacent); 0 = feral raw rock (the deepest chamber).
 * No hard seam — chambers mature continuously with elevation; these are the
 * sample points the gradient interpolates between.
 */
export interface StratumDef {
  id: StratumId;
  /** Display label for plaques / HUD / docs. */
  label: string;
  /** Chamber ceiling height in world-y (top of the band, 1u = 1m). */
  top: number;
  /** Chamber floor height in world-y (bottom of the band). */
  floor: number;
  /** Inner cavern radius at this depth — widens as the climb matures upward. */
  radius: number;
  /** 1 = composed senate stone, 0 = feral raw rock. Drives the W2 detail pass. */
  maturity: number;
}

// The crown floor sits at y=0; the floating platform underside at y≈-1. The cave
// hangs beneath it. The distant void grid (atmosphere/DistantGrid) is at y=-30, so
// the system lives in (-2 .. -28): five strata, ~5–6m chambers, widening with depth
// (a cavern, not a stack of equal rooms). Bedrock fades into the grid below.
export const STRATA: StratumDef[] = [
  { id: 'verge',     label: 'The Verge',     top: -2,  floor: -7,  radius: 13, maturity: 0.85 },
  { id: 'carved',    label: 'Carved Halls',  top: -7,  floor: -13, radius: 15, maturity: 0.6 },
  { id: 'roughwork', label: 'Rough Work',    top: -13, floor: -19, radius: 16, maturity: 0.35 },
  { id: 'mining',    label: 'The Mining',    top: -19, floor: -25, radius: 14, maturity: 0.12 },
  { id: 'bedrock',   label: 'Bedrock',       top: -25, floor: -29, radius: 10, maturity: 0 },
];

export const STRATA_BY_ID: Record<StratumId, StratumDef> = Object.fromEntries(
  STRATA.map((s) => [s.id, s])
) as Record<StratumId, StratumDef>;

/** Topmost (crown-adjacent) and deepest band extents — the system's vertical span. */
export const CAVE_TOP = STRATA[0].top; // -2
export const CAVE_FLOOR = STRATA[STRATA.length - 1].floor; // -29

/**
 * The throat — the Brain Archive's descending abyss (bible §2). The Brain zone
 * sits south of the hall (z=+24); its shaft drops straight down through every
 * stratum: memory-stacks near the surface giving way to raw rock at the bottom.
 * Centre is offset toward Brain so the descent reads as "entered on a bridge over
 * the descending abyss" (bible §5).
 */
export const THROAT = {
  /** Shaft centre (world x,z) — biased toward the Brain threshold (south, +z). */
  center: [0, 0, 9] as [number, number, number],
  /** Shaft inner radius — a clean bore near the crown. */
  radiusTop: 4,
  /** Widens into ragged raw rock at the bottom. */
  radiusBottom: 7,
  /** Where the bored shaft gives way to raw unworked rock (bible: "raw rock"). */
  rawRockY: -19,
} as const;

/**
 * The Mouth sightline aperture (bible §2). "A distant blade of pale daylight, far
 * above the highest chamber" — the light source for the whole world and the Mesh
 * portal. W1 owns GEOMETRY ONLY: the rock aperture and its frame, placed on the
 * vertical sightline up out of the crown's oculus. W4 owns the daylight shaft that
 * pours through it. Placed far above the dome (DOME_HEIGHT=18) so it reads as
 * "impossibly distant" (bible §7 hero shot).
 */
export const MOUTH = {
  /** Aperture centre — directly above the oculus on the world's vertical axis. */
  center: [0, 64, 0] as [number, number, number],
  /** The slit is a blade, not a circle: long on x, narrow on z. */
  width: 7,
  depth: 1.6,
  /** Thickness of the rock shelf the blade is cut through. */
  rockThickness: 3,
} as const;

/** Linear maturity at an arbitrary depth, interpolated between strata sample points. */
export function maturityAtDepth(y: number): number {
  if (y >= STRATA[0].top) return STRATA[0].maturity;
  if (y <= CAVE_FLOOR) return 0;
  for (const s of STRATA) {
    if (y <= s.top && y >= s.floor) {
      const t = (y - s.floor) / (s.top - s.floor); // 0 at floor, 1 at top
      return s.maturity * t + (s.maturity * 0.7) * (1 - t);
    }
  }
  return 0;
}
