import type { CartItem } from "../../core/types.ts";
import type { Tier } from "./discount.ts";
import { subtotalCents } from "./pricing.ts";
import { applyLoyaltyDiscount } from "./discount.ts";

export function checkoutTotalCents(items: CartItem[], tier: Tier): number {
  const subtotal = subtotalCents(items);
  return applyLoyaltyDiscount(subtotal, tier);
}
