// Construction site model — THE_CAVE_vision_bible.md §4: construction animations bound to
// real banked events, consuming W2's tending anchors. "Busy week: scaffolds rise. Quiet
// week: builders bank material and work slow." Growth is SLOW and event-driven — never a
// per-minute progress bar (§4 "Slow is sacred").
//
// Each construction site has a small number of build STAGES. A stage advances when the
// tending bank is spent (consumeBanked) — i.e. only when real describe/ingest material
// has accumulated. With an empty bank nothing rises (idle, not tending — bible §4).
//
// TENDING ANCHORS — W2 interface note (placeholder until W2 lands construction anchors):
// these ids follow W2's convention `${areaId}.${prop}${n}.${slot}` (see
// props/WorkstationCluster.tsx `${idPrefix}.bench${r}.seat${L}`). W2's construction kit
// (scaffold lattice / banked-stone piles, bible §9 W2) will publish `kind: 'stand'`
// anchors at these footprints; W3 consumes them by id. Until then we register our own so
// builders have a real place to stand and tend. Convention to confirm with W2:
//   build.scaffold1.tend  ·  brain.scaffold1.tend  (areaId-scoped, kind 'stand')

import type { AgentAnchor } from '../shared/anchors';
import { getAnchors, registerAnchors } from '../shared/anchors';
import { consumeBanked, getBankLevel } from '../shared/tendingBank';

/** One unit of banked material advances a stage by this much (0..1 within a stage). */
const STAGE_STEP = 1;
/** Stages per site before it is "built" and stops consuming. */
export const STAGES_PER_SITE = 4;
/** Minimum ms between stage advances — keeps growth unhurried even if the bank is full. */
const STAGE_COOLDOWN_MS = 2600;

export interface ConstructionSite {
  id: string;
  /** World-space footprint origin (the scaffold base). */
  position: [number, number, number];
  /** Where a builder stands to tend this site (the placeholder tending anchor). */
  tendAnchorId: string;
  /** 0 … STAGES_PER_SITE — integer stones set; fractional = stone rising this frame. */
  progress: number;
  /** Epoch ms of the last stage advance (cooldown gate). */
  lastAdvance: number;
}

// Two starter sites on bare stone where future wings rise (bible §4 "survey lines glow on
// bare stone where future wings will rise"). Positioned just outside the rotunda colonnade
// toward the Build (+x) and Brain (+z) thresholds so they read as new wings breaking ground.
export const SITES: ConstructionSite[] = [
  {
    id: 'build.scaffold1',
    position: [16.5, 0, -2.4],
    tendAnchorId: 'build.scaffold1.tend',
    progress: 0,
    lastAdvance: 0,
  },
  {
    id: 'brain.scaffold1',
    position: [2.4, 0, 16.5],
    tendAnchorId: 'brain.scaffold1.tend',
    progress: 0,
    lastAdvance: 0,
  },
];

let anchorsRegistered = false;

/**
 * Register the placeholder tending anchors (W2 convention) once. If W2 has already
 * published anchors at these ids, never overwrite them — consume theirs.
 */
export function ensureConstructionAnchors(): void {
  if (anchorsRegistered) return;
  anchorsRegistered = true;
  const existing = new Set(getAnchors().map((a) => a.id));
  const toAdd: AgentAnchor[] = [];
  for (const site of SITES) {
    if (existing.has(site.tendAnchorId)) continue;
    // Stand ~1.4u toward the rotunda centre from the footprint, facing the work.
    const [x, , z] = site.position;
    const len = Math.hypot(x, z) || 1;
    const tx = x - (x / len) * 1.4;
    const tz = z - (z / len) * 1.4;
    toAdd.push({
      id: site.tendAnchorId,
      areaId: site.id.startsWith('brain') ? 'brain' : 'build',
      kind: 'stand',
      position: [tx, 0, tz],
      facing: Math.atan2(x - tx, z - tz),
    });
  }
  if (toAdd.length) registerAnchors(toAdd);
}

/**
 * Spend banked material into construction. Called on a slow discrete tick (NOT per frame).
 * Advances at most one stage per cooldown so growth stays unhurried (bible §4). Returns
 * the site that advanced (for a one-shot stone-set flash), or null.
 */
export function tickConstruction(now: number): ConstructionSite | null {
  if (getBankLevel() <= 0) return null;
  // Pick the least-built unfinished site so wings rise together, not one-at-a-time.
  let target: ConstructionSite | null = null;
  for (const s of SITES) {
    if (s.progress >= STAGES_PER_SITE) continue;
    if (now - s.lastAdvance < STAGE_COOLDOWN_MS) continue;
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
