export type Tier = "bronze" | "silver" | "gold";

const TIER_RATES: Record<Tier, number> = {
  bronze: 0.05,
  silver: 0.1,
  gold: 0.15,
};

const GOLD_DISCOUNT_CAP_CENTS = 5000;

/**
 * Applies the loyalty-tier discount to a subtotal (in cents) and returns
 * the discounted total (in cents).
 */
export function applyLoyaltyDiscount(subtotalCents: number, tier: Tier): number {
  const rate = TIER_RATES[tier];
  const discount = subtotalCents * rate;
  return subtotalCents - discount;
}
