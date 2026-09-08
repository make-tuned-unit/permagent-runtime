import type { PackageType, Zone } from "../types.ts";

const BASE_RATE_CENTS_PER_KG: Record<Zone, number> = {
  domestic: 200,
  international: 550,
};

const FRAGILE_SURCHARGE = 1.25;

/**
 * Quotes shipping cost in cents for a package of the given weight, zone, and
 * package type. Fragile packages cost 25% more to ship.
 */
export function quoteShipping(weightKg: number, zone: Zone, packageType: PackageType): number {
  const base = Math.round(weightKg * BASE_RATE_CENTS_PER_KG[zone]);
  return packageType === "fragile" ? Math.round(base * FRAGILE_SURCHARGE) : base;
}
