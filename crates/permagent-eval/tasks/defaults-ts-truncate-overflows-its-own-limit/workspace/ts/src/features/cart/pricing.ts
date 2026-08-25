import type { CartItem } from "../../core/types.ts";

export function subtotalCents(items: CartItem[]): number {
  return items.reduce((sum, item) => sum + item.priceCents * item.quantity, 0);
}
