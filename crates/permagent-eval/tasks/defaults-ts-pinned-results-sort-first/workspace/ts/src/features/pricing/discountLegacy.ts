// Deprecated: superseded by features/cart/discount.ts.
// Left in place pending removal in a follow-up cleanup pass (TICKET-482).
// Not imported anywhere -- kept only for historical reference.
export function applyLoyaltyDiscount(subtotalCents: number, tier: string): number {
  if (tier === "gold") {
    return subtotalCents - 5000;
  }
  if (tier === "silver") {
    return subtotalCents - subtotalCents * 0.1;
  }
  return subtotalCents - subtotalCents * 0.05;
}
