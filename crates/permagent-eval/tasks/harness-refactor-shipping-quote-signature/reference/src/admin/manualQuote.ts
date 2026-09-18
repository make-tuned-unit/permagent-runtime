import { quoteShipping } from "../shipping/quote.ts";
import type { PackageType, Zone } from "../types.ts";

/** Support-desk tool: quote shipping for a one-off manual order. */
export function computeManualQuote(weightKg: number, zone: Zone, packageType: PackageType): number {
  return quoteShipping(weightKg, zone, packageType);
}
