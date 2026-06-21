// Construction site model — THE_CAVE_vision_bible.md §4: construction animations bound to
// real banked events, consuming W2's tending anchors. "Busy week: scaffolds rise. Quiet
// week: builders bank material and work slow." Growth is SLOW and event-driven — never a
// per-minute progress bar (§4 "Slow is sacred").
//
// Each construction site has a small number of build STAGES. A stage advances when the
// tending bank is spent (consumeBanked) — i.e. only when real describe/ingest material
// has accumulated. With an empty bank nothing rises (idle, not tending — bible §4).
//
// CONSTRUCTION ANCHORS — the W2 → W3 seam (now CLOSED). W1's SCAFFOLD_MOUNTS feed W2's
// construction kit (CaveConstruction.tsx), which publishes one frozen 'stand'-kind
// AgentAnchor per scaffold/banked mount under the stable id `${mount.id}.build`. W3
// consumes those FIVE ids verbatim (no placeholders, no hardcoded positions):
//   verge.westwing.scaffold.build · carved.gallery.scaffold.build ·
//   carved.staging.banked.build · roughwork.spur.scaffold.build · roughwork.cairn.banked.build
//
// DEFENSIVE: these anchors register only when the relevant lazy area chunk is mounted.
// Until then getAnchor(id) is undefined and that site is simply absent — workers fall
// back to idle/wander (behavior.ts), and nothing is drawn for it (ConstructionSite.tsx).
// We never invent a position; an unresolved site contributes nothing.

import { getAnchor } from '../shared/anchors';
import { consumeBanked, getBankLevel } from '../shared/tendingBank';
import type { StratumDef } from '../areas/strata/strata';

/** One unit of banked material advances a stage by this much (0..1 within a stage). */
const STAGE_STEP = 1;
/**
 * Stages a site needs to read as "built" (the visual maturity threshold). Growth is
 * NOT capped at this — the Carved Cave correction removes the old saturating ceiling
 * (the 16/20-stone cap) so live describe/ingest events keep raising sites past it. A
 * site stops being a *target* once built, but the count is unbounded for the ledger.
 */
export const STAGES_PER_SITE = 4;
/** Minimum ms between stage advances — keeps growth unhurried even if the bank is full. */
const STAGE_COOLDOWN_MS = 2600;

/** The five real construction 'stand' anchor ids W2 publishes (frozen seam). */
export const BUILD_ANCHOR_IDS = [
  'verge.westwing.scaffold.build',
  'carved.gallery.scaffold.build',
  'carved.staging.banked.build',
  'roughwork.spur.scaffold.build',
  'roughwork.cairn.banked.build',
] as const;

export interface ConstructionSite {
  id: string;
  /** Frozen W2 'stand' anchor id this site is built at (the worker's build position). */
  anchorId: string;
  /** 0 … STAGES_PER_SITE — integer stones set; fractional = stone rising this frame. */
  progress: number;
  /** Epoch ms of the last stage advance (cooldown gate). */
  lastAdvance: number;
}

// One site per published build anchor. Progress/cooldown state lives here; the world-space
// footprint is resolved on demand from the frozen anchor (resolveSite) — never literal.
export const SITES: ConstructionSite[] = BUILD_ANCHOR_IDS.map((anchorId) => ({
  id: anchorId,
  anchorId,
  progress: 0,
  lastAdvance: 0,
}));

/** A site whose anchor is currently registered (area chunk mounted), with its live pose. */
export interface ResolvedSite {
  site: ConstructionSite;
  position: [number, number, number];
  facing: number;
}

/**
 * Resolve a site against the frozen anchor registry. Returns null when the build area
 * chunk isn't mounted yet (anchor absent) — callers skip such sites defensively rather
 * than fabricate a position.
 */
export function resolveSite(site: ConstructionSite): ResolvedSite | null {
  const anchor = getAnchor(site.anchorId);
  if (!anchor) return null;
  return { site, position: anchor.position, facing: anchor.facing };
}

/** All sites whose build anchor is currently registered, with resolved poses. */
export function getResolvedSites(): ResolvedSite[] {
  const out: ResolvedSite[] = [];
  for (const s of SITES) {
    const r = resolveSite(s);
    if (r) out.push(r);
  }
  return out;
}

/**
 * Seed construction from the real derived strata (Carved Cave backfill). Sets each
 * site's progress to reflect the chamber it belongs to: fully-built chambers render at
 * STAGES_PER_SITE, the deepest actively-forming (verge) chamber takes the curve's carve
 * fraction. Called ONCE at mount, before live ticks. Idempotent: it sets, not adds.
 *
 * Mapping: site ids are prefixed by their stratum name (`verge.*`, `carved.*`,
 * `roughwork.*`). The derived strata are top→deep (index 0 = verge = newest = forming).
 * We match a site to its chamber by index parity over the available chambers; if there
 * are more sites than chambers (shallow cave), the surplus sites stay at survey (0) —
 * we never invent built structure the memory count doesn't support.
 */
export function seedConstruction(strata: StratumDef[]): void {
  if (strata.length === 0) return;
  for (const s of SITES) {
    // The forming chamber is the verge (strata[0]); its carve drives the verge sites.
    // Built (deeper) chambers → full. We key by the site's stratum prefix when it maps
    // to a known band; otherwise fall back to "deeper = built".
    const prefix = s.id.split('.')[0]; // 'verge' | 'carved' | 'roughwork'
    let carve = 1;
    if (prefix === 'verge') {
      // Verge is the actively-forming chamber — partial carve from the curve.
      carve = strata[0]?.carve ?? 0;
    } else {
      // Deeper chambers are fully built once the cave is that deep; if the cave is
      // too shallow to contain this band, leave the site at bare survey (honest).
      const bandIndex = prefix === 'carved' ? 1 : 2;
      carve = bandIndex < strata.length ? 1 : 0;
    }
    s.progress = carve * STAGES_PER_SITE;
    s.lastAdvance = 0; // live growth can resume immediately
  }
}

/**
 * Spend banked material into construction. Called on a slow discrete tick (NOT per frame).
 * Advances at most one stage per cooldown so growth stays unhurried (bible §4). Returns
 * the site that advanced (for a one-shot stone-set flash), or null. Only sites whose
 * anchor is currently registered are eligible — an unmounted area can't rise.
 *
 * Carved Cave: there is no longer a hard saturation cap. Sites still prefer the
 * least-built one (wings rise together), but live describe/ingest events keep raising
 * every site past STAGES_PER_SITE — growth continues for the life of the session.
 */
export function tickConstruction(now: number): ConstructionSite | null {
  if (getBankLevel() <= 0) return null;
  // Pick the least-built, currently-resolvable site so wings rise together. No upper
  // clamp: a "built" site can still accept more real material (the cap is gone).
  let target: ConstructionSite | null = null;
  for (const s of SITES) {
    if (now - s.lastAdvance < STAGE_COOLDOWN_MS) continue;
    if (!getAnchor(s.anchorId)) continue; // area not mounted — can't build here
    if (!target || s.progress < target.progress) target = s;
  }
  if (!target) return null;
  if (consumeBanked(1) === 0) return null;
  target.progress += STAGE_STEP; // unbounded — live growth past the old cap
  target.lastAdvance = now;
  return target;
}

/** Total stones set across all sites (evidence/ledger). */
export function getConstructionProgress(): { built: number; total: number } {
  let built = 0;
  for (const s of SITES) built += s.progress;
  return { built, total: SITES.length * STAGES_PER_SITE };
}

/** Reset (dev/time-lapse teardown). */
export function resetConstruction(): void {
  for (const s of SITES) {
    s.progress = 0;
    s.lastAdvance = 0;
  }
}
