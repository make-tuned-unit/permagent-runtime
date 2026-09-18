import type { Zone } from "../types.ts";

const BASE_RATE_CENTS_PER_KG: Record<Zone, number> = {
  domestic: 200,
  international: 550,
};

/**
 * Quotes shipping cost in cents for a package of the given weight and zone.
 */
export function quoteShipping(weightKg: number, zone: Zone): number {
  return Math.round(weightKg * BASE_RATE_CENTS_PER_KG[zone]);
}
