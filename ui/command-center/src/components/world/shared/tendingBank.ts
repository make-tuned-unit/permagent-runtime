// Tending bank — the REAL banked-events ledger that drives tending + construction.
// THE_CAVE_vision_bible.md §4: "The Brain is the quarry. Every Librarian-described
// memory is a quarried stone; Reader OCR is ore. Construction rate is driven by real
// events — never a usage progress bar." §4 third agent state: tending is driven by
// banked describe/ingest events. "No bank means idle, not tending."
//
// HONESTY (bible §8 / sim-state clamp): a unit of banked material is deposited ONLY by
// a real daemon event — `librarian_describe_completed` (a quarried stone) or
// `memory_added` (ingest, the ore). Replayed /events history (timestamp before the
// page mounted) is ignored — banking only counts work that happened while watching, so
// the bank can never be faked by a stale buffer. The dev time-lapse driver deposits
// through the SAME door but tags its units `dev` so evidence can prove provenance.
//
// This module is the single source for: how much material is banked (the ledger), and
// whether tending should be happening at all (bankLevel > 0). It is NOT frozen — it is
// a W3 module — but it deliberately mirrors the agentStatus external-store shape so the
// honesty boundary reads the same way everywhere.

import { useSyncExternalStore } from 'react';

/** Provenance of a banked unit — evidence must show 'daemon' for real-run captures. */
export type BankSource = 'daemon' | 'dev';

/** A single banked unit of construction material (one real quarried stone / ore). */
export interface BankedUnit {
  /** Monotonic id (deposit order). */
  seq: number;
  /** The real event that deposited it. */
  kind: 'describe' | 'ingest';
  /** Memory key / payload id from the daemon event (evidence trail). */
  ref: string;
  /** Epoch ms when banked. */
  at: number;
  source: BankSource;
}

/** What anyone reading the bank sees (snapshot — referentially stable until a change). */
export interface BankSnapshot {
  /** Units available to consume (not yet spent on construction). */
  level: number;
  /** Total ever banked this session (the ledger length, including spent). */
  totalBanked: number;
  /** Total consumed by construction. */
  totalSpent: number;
  /** The most recent deposits (newest last), capped — for the dev ledger overlay. */
  recent: BankedUnit[];
}

const RECENT_CAP = 12;

let seq = 0;
let totalBanked = 0;
let totalSpent = 0;
const available: BankedUnit[] = []; // FIFO queue of unspent units
const recent: BankedUnit[] = [];
let snapshot: BankSnapshot = { level: 0, totalBanked: 0, totalSpent: 0, recent: [] };
const listeners = new Set<() => void>();

function publish(): void {
  snapshot = {
    level: available.length,
    totalBanked,
    totalSpent,
    recent: recent.slice(),
  };
  listeners.forEach((l) => l());
}

/**
 * Deposit one unit of material from a REAL event. Called by the /events binding
 * (source 'daemon') or the dev time-lapse driver (source 'dev'). There is no other
 * way to add to the bank — placeholder/idle activity never deposits.
 */
export function bankEvent(kind: BankedUnit['kind'], ref: string, source: BankSource): void {
  const unit: BankedUnit = { seq: seq++, kind, ref, at: Date.now(), source };
  available.push(unit);
  totalBanked++;
  recent.push(unit);
  if (recent.length > RECENT_CAP) recent.shift();
  publish();
}

/**
 * Backfill the bank from REAL history (Carved Cave correction). Sets the running
 * totals directly from the one-shot /api/world/strata seed — no per-unit bankEvent
 * loop (that would fabricate `recent` deposits and spam the ledger). `totalBanked`
 * becomes the real lifetime memory count; `level` (available to construct) is set so
 * the seeded structure reads as already-built without inventing live events.
 *
 * Honesty: this is structure HISTORY, not live ambience — it reflects work that truly
 * happened. Live `bankEvent` deposits (in-session growth) are untouched and continue
 * to accrue on top. Idempotent: called once at mount before any live event.
 */
export function hydrateBank({ total_memories }: { total_memories: number }): void {
  const total = Math.max(0, Math.floor(total_memories));
  // Reflect lifetime volume as banked history. `available` stays empty — the seeded
  // construction is set directly (seedConstruction), so we don't need consumable units
  // sitting in the queue; we only record the real totals for the ledger/HUD.
  totalBanked = total;
  totalSpent = total;
  available.length = 0;
  // Preserve any deposit seq already advanced; do not reset live counters.
  publish();
}

/**
 * Spend up to `n` banked units (construction consumes the bank). Returns how many were
 * actually spent — fewer than `n` if the bank ran low. Spending the last unit drops the
 * bank to 0 and tending stops (idle, not tending — bible §4).
 */
export function consumeBanked(n = 1): number {
  const take = Math.min(n, available.length);
  if (take === 0) return 0;
  available.splice(0, take);
  totalSpent += take;
  publish();
  return take;
}

/** True iff there is banked material to tend (bible §4: no bank ⇒ idle, not tending). */
export function hasBankedWork(): boolean {
  return available.length > 0;
}

/** Non-hook read for useFrame loops (no per-frame React work, no allocation). */
export function getBankLevel(): number {
  return available.length;
}

/** Full snapshot accessor (non-hook). */
export function getBankSnapshot(): BankSnapshot {
  return snapshot;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** React binding for the dev ledger overlay and the tending-active gate. */
export function useTendingBank(): BankSnapshot {
  return useSyncExternalStore(subscribe, () => snapshot);
}

/** Test/dev reset — clears the ledger (used by the time-lapse driver on teardown). */
export function resetBank(): void {
  seq = 0;
  totalBanked = 0;
  totalSpent = 0;
  available.length = 0;
  recent.length = 0;
  publish();
}
