// Derived-strata store — the single source of truth for the Carved Cave's shape.
// The one-shot backfill (agents/stateSources.tsx) fetches /api/world/strata, runs
// deriveStrata, and publishes here ONCE. Chambers (StrataChambers), the descent rail
// (DescentMode), the plaques (StratumPlaque), and construction seeding all read this
// same set, so they can never disagree about how deep the cave is or what each chamber
// holds. Mirrors the tendingBank / tourState external-store shape (honesty boundary
// reads the same everywhere). No per-frame React work — read via getStrata() in loops.

import { useSyncExternalStore } from 'react';
import {
  deriveStrata,
  caveFloorFor,
  CAVE_TOP,
  type StratumDef,
  type StrataSlice,
} from './strata';

// Before the backfill lands, the cave is a single shallow chamber with no real data —
// honest-empty (the descent still works, the plaque reads zero). Replaced wholesale on
// the first successful seed.
const EMPTY_STRATA = deriveStrata(
  [{ start: '', end: '', memory_count: 0, described_count: 0 }],
  0
);

let strata: StratumDef[] = EMPTY_STRATA;
let totalMemories = 0;
let firstMemoryAt: string | null = null;
let seeded = false;
const listeners = new Set<() => void>();

/** Non-hook read for useFrame loops + module callers (no allocation). */
export function getStrata(): StratumDef[] {
  return strata;
}

/** Derived cave floor of the current band set (bottom of the deepest chamber). */
export function getCaveFloor(): number {
  return caveFloorFor(strata);
}

/** Top of the shallowest chamber — the verge ceiling (re-exported for consumers). */
export function getCaveTop(): number {
  return CAVE_TOP;
}

export function isStrataSeeded(): boolean {
  return seeded;
}

export function getStrataMeta(): { totalMemories: number; firstMemoryAt: string | null } {
  return { totalMemories, firstMemoryAt };
}

/**
 * Seed the derived strata from real /api/world/strata data. Called ONCE by the
 * backfill mount effect. `slices` arrive oldest-first; `carving` is the depth curve's
 * carve fraction for the actively-forming (verge) chamber.
 */
export function seedStrata(
  slices: StrataSlice[],
  carving: number,
  total: number,
  first: string | null
): void {
  strata = deriveStrata(slices.length > 0 ? slices : EMPTY_STRATA_SLICES, carving);
  totalMemories = total;
  firstMemoryAt = first;
  seeded = true;
  listeners.forEach((l) => l());
}

// Fallback slice when the daemon honestly returns zero slices (empty Brain) — keeps a
// single shallow chamber rather than an empty void.
const EMPTY_STRATA_SLICES: StrataSlice[] = [
  { start: '', end: '', memory_count: 0, described_count: 0 },
];

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** React binding — re-renders chambers/plaques when the strata seed lands. */
export function useStrata(): StratumDef[] {
  return useSyncExternalStore(subscribe, getStrata);
}

/** Test/dev reset. */
export function resetStrata(): void {
  strata = EMPTY_STRATA;
  totalMemories = 0;
  firstMemoryAt = null;
  seeded = false;
  listeners.forEach((l) => l());
}
