import { quoteShipping } from "../shipping/quote.ts";
import type { Order } from "../types.ts";

/** Estimated total shipping cost (cents) for every order in a cart preview. */
export function estimateCartShipping(orders: Order[]): number {
  return orders.reduce((sum, o) => sum + quoteShipping(o.weightKg, o.zone, o.packageType), 0);
}
