import { quoteShipping } from "../shipping/quote.ts";
import type { Zone } from "../types.ts";

/** Support-desk tool: quote shipping for a one-off manual order. */
export function computeManualQuote(weightKg: number, zone: Zone): number {
  return quoteShipping(weightKg, zone);
}
