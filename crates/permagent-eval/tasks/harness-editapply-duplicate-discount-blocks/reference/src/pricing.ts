// ---------------------------------------------------------------------------
// Checkout tier discounts
// ---------------------------------------------------------------------------
//
// Published loyalty tier discount table (finance-approved, do not change
// without sign-off):
//
//   tier      | percent off | cap
//   ----------|-------------|--------
//   bronze    | 5%          | none
//   silver    | 15%         | $20.00
//   gold      | 20%         | $50.00
//   platinum  | 30%         | $100.00
//
// Each tier below validates its input, computes the raw percentage discount,
// applies the tier's cap (if any), and returns a DiscountResult describing
// the computation. The four tier functions are intentionally structured the
// same way (validate -> compute -> cap -> assemble -> log) so that reading
// one tells you how to read the rest; historically that has meant a fix
// meant for one tier's constant getting applied to the wrong block by a
// careless text search, since the surrounding code is nearly identical
// tier-to-tier and some of the actual numbers repeat across tiers.
//
// Changelog (informational only):
//   * 2023-02: initial bronze/silver/gold tiers shipped.
//   * 2023-11: platinum tier added for the top loyalty bracket.
//   * 2024-06: silver cap raised from $15 to $20.
//   * 2024-09: gold cap raised from $35 to $50.

export interface DiscountResult {
  tier: string;
  originalCents: number;
  percentOff: number;
  discountCents: number;
  cappedAt: number | null;
  finalCents: number;
}

export interface AuditEntry {
  tier: string;
  discountCents: number;
  timestampMs: number;
}

// A process-local audit trail of computed discounts. Production code sends
// these to the ledger service; this fixture just accumulates them in memory
// so the calculation functions have a realistic side effect to reason about.
export const auditLog: AuditEntry[] = [];

/**
 * Validates that a subtotal is a sane, finite, non-negative amount of cents.
 * Throws a descriptive error otherwise so a bad upstream computation fails
 * fast instead of silently producing a nonsense discount.
 */
function validateSubtotal(subtotalCents: number): void {
  if (!Number.isFinite(subtotalCents)) {
    throw new Error(`invalid subtotal: ${subtotalCents} is not finite`);
  }
  if (subtotalCents < 0) {
    throw new Error(`invalid subtotal: ${subtotalCents} is negative`);
  }
}

/** Formats a cents amount as a dollar string, e.g. `1234` -> `"$12.34"`. */
export function formatCents(cents: number): string {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  return `${sign}$${(abs / 100).toFixed(2)}`;
}

/**
 * Records a computed discount in the in-memory audit log. Kept as a no-op
 * beyond the push so call sites read the same in every tier function.
 */
function recordAudit(tier: string, discountCents: number): void {
  auditLog.push({ tier, discountCents, timestampMs: Date.now() });
}

/**
 * Bronze tier: 5% off, uncapped. Bronze is the entry loyalty tier and never
 * carries a discount cap -- large bronze orders are rare enough that finance
 * decided the uncapped exposure is acceptable.
 */
export function computeBronzeDiscount(subtotalCents: number): DiscountResult {
  validateSubtotal(subtotalCents);
  const percentOff = 0.05;
  let discountCents = Math.round(subtotalCents * percentOff);
  const capCents: number | null = null;
  if (capCents !== null && discountCents > capCents) {
    discountCents = capCents;
  }
  const finalCents = subtotalCents - discountCents;
  const result: DiscountResult = {
    tier: "bronze",
    originalCents: subtotalCents,
    percentOff,
    discountCents,
    cappedAt: capCents,
    finalCents,
  };
  recordAudit("bronze", discountCents);
  return result;
}

/**
 * Silver tier: 15% off, capped at $20.00. Silver is the mid loyalty tier;
 * the cap was raised from $15 to $20 in the 2024-06 pricing pass.
 */
export function computeSilverDiscount(subtotalCents: number): DiscountResult {
  validateSubtotal(subtotalCents);
  const percentOff = 0.15;
  let discountCents = Math.round(subtotalCents * percentOff);
  const capCents: number | null = 2000;
  if (capCents !== null && discountCents > capCents) {
    discountCents = capCents;
  }
  const finalCents = subtotalCents - discountCents;
  const result: DiscountResult = {
    tier: "silver",
    originalCents: subtotalCents,
    percentOff,
    discountCents,
    cappedAt: capCents,
    finalCents,
  };
  recordAudit("silver", discountCents);
  return result;
}

/**
 * Gold tier: 20% off, capped at $50.00. Gold is the upper-mid loyalty tier;
 * the cap was raised from $35 to $50 in the 2024-09 pricing pass.
 */
export function computeGoldDiscount(subtotalCents: number): DiscountResult {
  validateSubtotal(subtotalCents);
  const percentOff = 0.2;
  let discountCents = Math.round(subtotalCents * percentOff);
  const capCents: number | null = 5000;
  if (capCents !== null && discountCents > capCents) {
    discountCents = capCents;
  }
  const finalCents = subtotalCents - discountCents;
  const result: DiscountResult = {
    tier: "gold",
    originalCents: subtotalCents,
    percentOff,
    discountCents,
    cappedAt: capCents,
    finalCents,
  };
  recordAudit("gold", discountCents);
  return result;
}

/**
 * Platinum tier: 30% off, capped at $100.00. Platinum is the top loyalty
 * tier, added in the 2023-11 pricing pass.
 */
export function computePlatinumDiscount(subtotalCents: number): DiscountResult {
  validateSubtotal(subtotalCents);
  const percentOff = 0.3;
  let discountCents = Math.round(subtotalCents * percentOff);
  const capCents: number | null = 10000;
  if (capCents !== null && discountCents > capCents) {
    discountCents = capCents;
  }
  const finalCents = subtotalCents - discountCents;
  const result: DiscountResult = {
    tier: "platinum",
    originalCents: subtotalCents,
    percentOff,
    discountCents,
    cappedAt: capCents,
    finalCents,
  };
  recordAudit("platinum", discountCents);
  return result;
}

export type TierName = "bronze" | "silver" | "gold" | "platinum";

export const TIER_ORDER: TierName[] = ["bronze", "silver", "gold", "platinum"];

/** Dispatches to the correct per-tier calculation by tier name. */
export function computeDiscount(tier: TierName, subtotalCents: number): DiscountResult {
  switch (tier) {
    case "bronze":
      return computeBronzeDiscount(subtotalCents);
    case "silver":
      return computeSilverDiscount(subtotalCents);
    case "gold":
      return computeGoldDiscount(subtotalCents);
    case "platinum":
      return computePlatinumDiscount(subtotalCents);
    default: {
      const exhaustive: never = tier;
      throw new Error(`unknown tier: ${exhaustive}`);
    }
  }
}

/** The tier immediately above `tier`, or null if `tier` is already top. */
export function nextTierUp(tier: TierName): TierName | null {
  const idx = TIER_ORDER.indexOf(tier);
  if (idx === -1 || idx === TIER_ORDER.length - 1) {
    return null;
  }
  return TIER_ORDER[idx + 1];
}

/**
 * Describes, in a short human-readable sentence, how much a tier saved on a
 * given subtotal. Used by the order-confirmation email template.
 */
export function describeSavings(tier: TierName, subtotalCents: number): string {
  const result = computeDiscount(tier, subtotalCents);
  const capNote = result.cappedAt !== null && result.discountCents === result.cappedAt
    ? ` (capped at ${formatCents(result.cappedAt)})`
    : "";
  return `${tier} membership saved you ${formatCents(result.discountCents)}${capNote} on this order.`;
}

/**
 * Returns the best (largest) discount across all tiers for a subtotal. Used
 * by the "which membership pays for itself fastest" comparison widget on the
 * pricing page.
 */
export function bestDiscountAcrossTiers(subtotalCents: number): DiscountResult {
  let best: DiscountResult | null = null;
  for (const tier of TIER_ORDER) {
    const result = computeDiscount(tier, subtotalCents);
    if (best === null || result.discountCents > best.discountCents) {
      best = result;
    }
  }
  // TIER_ORDER is non-empty, so `best` is always assigned by the loop above.
  return best as DiscountResult;
}

/**
 * Returns the total discount cents saved across a full audit log, grouped by
 * tier. Used by the weekly finance report.
 */
export function summarizeAuditLog(entries: AuditEntry[]): Record<string, number> {
  const totals: Record<string, number> = {};
  for (const entry of entries) {
    totals[entry.tier] = (totals[entry.tier] ?? 0) + entry.discountCents;
  }
  return totals;
}

/**
 * The number of audit entries recorded for a given tier so far this process.
 * Used by the admin dashboard's "discounts applied today" counter.
 */
export function countAuditEntriesForTier(entries: AuditEntry[], tier: TierName): number {
  let count = 0;
  for (const entry of entries) {
    if (entry.tier === tier) {
      count += 1;
    }
  }
  return count;
}

/**
 * Clears the in-memory audit log. Only ever called from test setup and from
 * the nightly rollover job that persists the day's entries to the ledger
 * service before starting a fresh in-memory log.
 */
export function resetAuditLog(): void {
  auditLog.length = 0;
}

// ---------------------------------------------------------------------------
// Promotional stacking
// ---------------------------------------------------------------------------
//
// A small number of seasonal promotions can stack on top of a loyalty tier
// discount (never on top of each other). The promotion is applied to the
// post-tier-discount subtotal, and the combined discount is still subject to
// an overall ceiling so a stacked promo can never wipe out more than 50% of
// the original order -- that ceiling exists to protect against a
// misconfigured seasonal promo combined with platinum from zeroing an order
// out entirely.

export interface StackedDiscountResult extends DiscountResult {
  promoCents: number;
  combinedDiscountCents: number;
}

const MAX_COMBINED_DISCOUNT_FRACTION = 0.5;

/**
 * Applies a flat seasonal promotion (in cents) on top of a tier discount,
 * respecting the combined-discount ceiling.
 */
export function applyStackedPromo(
  tier: TierName,
  subtotalCents: number,
  promoCents: number,
): StackedDiscountResult {
  validateSubtotal(subtotalCents);
  if (promoCents < 0) {
    throw new Error(`invalid promo amount: ${promoCents} is negative`);
  }
  const tierResult = computeDiscount(tier, subtotalCents);
  const rawCombined = tierResult.discountCents + promoCents;
  const ceiling = Math.floor(subtotalCents * MAX_COMBINED_DISCOUNT_FRACTION);
  const combinedDiscountCents = Math.min(rawCombined, ceiling);
  return {
    ...tierResult,
    promoCents,
    combinedDiscountCents,
    finalCents: subtotalCents - combinedDiscountCents,
  };
}

/**
 * Whether a given combined discount result actually hit the 50% ceiling
 * (i.e. the seasonal promo was partially or fully clipped). Used to log a
 * warning so finance notices when a promo configuration is too aggressive.
 */
export function wasCeilingClipped(result: StackedDiscountResult): boolean {
  return result.combinedDiscountCents < result.discountCents + result.promoCents;
}

/**
 * A tiny amount of home-grown documentation-by-example: these mirror the
 * hand-checked numbers in the pricing spec doc so a reader can eyeball that
 * the constants above still line up, without needing to run the test suite.
 *
 *   computeBronzeDiscount(10_000)   -> 5%  of $100.00 = $5.00,  uncapped
 *   computeSilverDiscount(10_000)   -> 15% of $100.00 = $15.00, under the $20 cap
 *   computeGoldDiscount(10_000)     -> 20% of $100.00 = $20.00, under the $50 cap
 *   computePlatinumDiscount(10_000) -> 30% of $100.00 = $30.00, under the $100 cap
 *
 *   computeSilverDiscount(200_000)  -> 15% of $2000.00 = $300.00, capped at $20.00
 *   computeGoldDiscount(500_000)    -> 20% of $5000.00 = $1000.00, capped at $50.00
 */
