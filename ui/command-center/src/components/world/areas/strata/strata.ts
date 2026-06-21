// Strata registry — THE_CAVE_vision_bible.md §2 ("Depth is time") + §9 W1 scope.
// The vertical cave system beneath the crown. The crown (rotunda at y=0, dome to
// y≈18) is the finished forum nearest the light; descending in -y walks BACKWARD
// through the agents' maturation — top strata maturer, deepest strata rawest rock.
//
// CARVED CAVE CORRECTION (Phase 1): the cave is no longer a fixed 5-band const. Its
// DEPTH is a deterministic function of the real memory count (the depth curve below),
// and each chamber is an equal TIME-SLICE of the lived history — oldest deepest, newest
// at the verge. `deriveStrata(slices, carving)` generates the bands from real /api/world/
// strata data; the old saturating 5-stone cap is gone (the bug this corrects). Labels
// cycle the five poetic strata names; the PLAQUE leads with the real date range + counts
// (knob 1 — data is the truth, names are flavor, scales to ANY depth).
//
// W1-owned. Real scale: 1 unit = 1 metre (WORLD_VIEW_BIBLE.md §1).

/**
 * Legacy stratum ids — retained for the W1 SCAFFOLD_MOUNTS blockout table
 * (areas/strata/anchors.ts), which is a FIXED construction-seam registry keyed by
 * the original five band names. That table is separate from the now-dynamic chambers
 * (the carve seam is frozen across W2/W3); it keeps the literal-union id type.
 */
export type StratumId = 'verge' | 'carved' | 'roughwork' | 'mining' | 'bedrock';

/** A single time-slice of memory history from /api/world/strata. */
export interface StrataSlice {
  start: string;
  end: string;
  memory_count: number;
  described_count: number;
}

/**
 * Maturity grades the material gradient (bible §3, "two strata one gradient").
 * 1 = forum-clean (crown-adjacent); 0 = feral raw rock (the deepest chamber).
 * No hard seam — chambers mature continuously with elevation; these are the
 * sample points the gradient interpolates between.
 */
export interface StratumDef {
  /** Stable id, top (maturest) to deepest: `stratum.0` is the verge. */
  id: string;
  /** Poetic display label (cycles the five names past 5 chambers). */
  label: string;
  /** Chamber ceiling height in world-y (top of the band, 1u = 1m). */
  top: number;
  /** Chamber floor height in world-y (bottom of the band). */
  floor: number;
  /** Inner cavern radius at this depth — widens as the climb matures upward. */
  radius: number;
  /** 1 = composed senate stone, 0 = feral raw rock. Drives the W2 detail pass. */
  maturity: number;
  /** Real plaque data — the date range + counts this chamber represents. */
  dateRange: { start: string; end: string };
  memoryCount: number;
  describedCount: number;
  /**
   * 0..1 carve progress for the deepest, actively-forming chamber (the verge of
   * the dig). 1 for fully-built chambers. Drives seedConstruction + the W2 pass.
   */
  carve: number;
}

// ── THE DEPTH CURVE (rulings, tuned to 1457 memories) ──────────────────────
// chamberThresholds → 1..6 chambers. N_full = thresholds STRICTLY exceeded by the
// total (clamp 0..6). The actively-forming chamber is never maxed — its carve is a
// LOG-interpolated fraction between the last-passed and next threshold.
export const CHAMBER_THRESHOLDS = [10, 60, 200, 500, 1200, 3000] as const;
export const MAX_CHAMBERS = 6;

/** The five poetic strata names — cycled past 5 chambers (knob 1). Atmosphere only. */
const STRATUM_NAMES = ['The Verge', 'Carved Halls', 'Rough Work', 'The Mining', 'Bedrock'] as const;

export interface DepthResult {
  /** Fully-formed chambers (thresholds strictly exceeded), 0..6. */
  nFull: number;
  /** Total slices to request/render = nFull + (carving chamber), clamped 1..6. */
  slices: number;
  /** 0..1 LOG-interpolated carve fraction of the actively-forming chamber. */
  carving: number;
}

/**
 * The locked depth curve. Pure f(total) → how deep the cave is. Guards every edge:
 * total=0 → one shallow carving chamber (carving 0); never divides by zero; never
 * saturates. log2(1+x) keeps the carving curve defined at x=0.
 */
export function computeDepth(total: number): DepthResult {
  const t = Math.max(0, Math.floor(total));
  // N_full = thresholds STRICTLY exceeded, clamped 0..6.
  let nFull = 0;
  for (const thr of CHAMBER_THRESHOLDS) {
    if (t > thr) nFull++;
  }
  nFull = Math.min(MAX_CHAMBERS, nFull);

  // Carving fraction of the chamber currently forming (the one above nFull).
  // LOG-interpolate from threshold[nFull-1] → threshold[nFull]. At the deepest
  // possible chamber (nFull == MAX) there is no "next" — it's full (carving 1).
  let carving: number;
  if (nFull >= MAX_CHAMBERS) {
    carving = 1;
  } else {
    const lo = nFull === 0 ? 0 : CHAMBER_THRESHOLDS[nFull - 1];
    const hi = CHAMBER_THRESHOLDS[nFull];
    // Logarithmic position of t within (lo, hi]. log2(1+x) guards x=0.
    const span = Math.log2(1 + (hi - lo));
    const pos = Math.log2(1 + Math.max(0, t - lo));
    carving = span > 0 ? Math.min(1, Math.max(0, pos / span)) : 0;
  }

  // Total chambers rendered: the full ones plus the forming one (always ≥1 so an
  // empty Brain still shows a single shallow chamber to descend into).
  const slices = Math.min(MAX_CHAMBERS, Math.max(1, nFull + (nFull < MAX_CHAMBERS ? 1 : 0)));
  return { nFull, slices, carving };
}

// ── Geometry tuning (derived bands) ────────────────────────────────────────
/** Top of the shallowest chamber (crown-adjacent). Unchanged from the bible. */
export const CAVE_TOP = -2;
/** Each chamber's vertical extent (1u = 1m). ~5.5m cavern per the bible. */
const BAND_HEIGHT = 5.5;
// Radius ramp: wide near the crown (mature, open halls), narrowing into the raw
// bore at the bottom — generates the old 13→10 ramp at any depth.
const RADIUS_TOP = 13;
const RADIUS_BOTTOM = 10;

/**
 * Generate N derived strata bands from real time-slices. `slices` arrive oldest-first
 * (deepest band first). We render top→deep, so the LAST slice is the verge (newest) and
 * the FIRST slice is the deepest (oldest). The actively-forming chamber is the verge — it
 * takes the `carving` progress; every deeper chamber is fully built.
 *
 * Maturity gradient runs 0.85 (verge) → 0 (bedrock). If described data is absent
 * everywhere (knob 2), the gradient still drives the silhouette but carved/raw plaque
 * distinction degrades to count-only — no fabricated maturity.
 */
export function deriveStrata(slices: StrataSlice[], carving: number): StratumDef[] {
  const n = Math.max(1, slices.length);
  // slices[] is oldest→newest. Build top→deep: index 0 = verge (newest), reading
  // slices from the END. y descends from CAVE_TOP.
  const out: StratumDef[] = [];
  for (let i = 0; i < n; i++) {
    const top = CAVE_TOP - i * BAND_HEIGHT;
    const floor = top - BAND_HEIGHT;
    // Depth fraction: 0 at the verge (i=0), 1 at the deepest band.
    const depthT = n === 1 ? 0 : i / (n - 1);
    const radius = RADIUS_TOP + (RADIUS_BOTTOM - RADIUS_TOP) * depthT;
    const maturity = 0.85 * (1 - depthT); // 0.85 verge → 0 bedrock
    // Newest slice (the verge) is at the END of the oldest-first array.
    const slice = slices[n - 1 - i] ?? {
      start: '',
      end: '',
      memory_count: 0,
      described_count: 0,
    };
    out.push({
      id: `stratum.${i}`,
      // Cycle the five poetic names; data leads on the plaque (knob 1).
      label: STRATUM_NAMES[i % STRATUM_NAMES.length],
      top,
      floor,
      radius,
      maturity,
      dateRange: { start: slice.start, end: slice.end },
      memoryCount: slice.memory_count,
      describedCount: slice.described_count,
      // The verge (i=0) is the actively-forming chamber → carve = curve carving.
      // Every deeper chamber is already built.
      carve: i === 0 ? Math.min(1, Math.max(0, carving)) : 1,
    });
  }
  return out;
}

/** Derived CAVE_FLOOR for a given band set — bottom of the deepest band. */
export function caveFloorFor(strata: StratumDef[]): number {
  if (strata.length === 0) return CAVE_TOP - BAND_HEIGHT;
  return strata[strata.length - 1].floor;
}

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

/**
 * Linear maturity at an arbitrary depth, interpolated between a band set's sample
 * points (same algorithm as before, dynamic input). Used by the W2 material pass.
 */
export function maturityAtDepth(y: number, strata: StratumDef[]): number {
  if (strata.length === 0) return 0;
  if (y >= strata[0].top) return strata[0].maturity;
  const floorY = caveFloorFor(strata);
  if (y <= floorY) return 0;
  for (const s of strata) {
    if (y <= s.top && y >= s.floor) {
      const t = (y - s.floor) / (s.top - s.floor); // 0 at floor, 1 at top
      return s.maturity * t + s.maturity * 0.7 * (1 - t);
    }
  }
  return 0;
}
