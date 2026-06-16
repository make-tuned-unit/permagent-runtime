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

/** One unit of banked material advances a stage by this much (0..1 within a stage). */
const STAGE_STEP = 1;
/** Stages per site before it is "built" and stops consuming. */
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
 * Spend banked material into construction. Called on a slow discrete tick (NOT per frame).
 * Advances at most one stage per cooldown so growth stays unhurried (bible §4). Returns
 * the site that advanced (for a one-shot stone-set flash), or null. Only sites whose
 * anchor is currently registered are eligible — an unmounted area can't rise.
 */
export function tickConstruction(now: number): ConstructionSite | null {
  if (getBankLevel() <= 0) return null;
  // Pick the least-built unfinished, currently-resolvable site so wings rise together.
  let target: ConstructionSite | null = null;
  for (const s of SITES) {
    if (s.progress >= STAGES_PER_SITE) continue;
    if (now - s.lastAdvance < STAGE_COOLDOWN_MS) continue;
    if (!getAnchor(s.anchorId)) continue; // area not mounted — can't build here
    if (!target || s.progress < target.progress) target = s;
  }
  if (!target) return null;
  if (consumeBanked(1) === 0) return null;
  target.progress = Math.min(STAGES_PER_SITE, target.progress + STAGE_STEP);
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
