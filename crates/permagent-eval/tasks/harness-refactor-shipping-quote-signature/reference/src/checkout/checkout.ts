import { quoteShipping } from "../shipping/quote.ts";
import type { Order } from "../types.ts";

export interface CheckoutSummary {
  itemsTotalCents: number;
  shippingCents: number;
  grandTotalCents: number;
}

export function buildCheckoutSummary(order: Order): CheckoutSummary {
  const itemsTotalCents = 5000; // placeholder demo total, not under test
  const shippingCents = quoteShipping(order.weightKg, order.zone, order.packageType);
  return {
    itemsTotalCents,
    shippingCents,
    grandTotalCents: itemsTotalCents + shippingCents,
  };
}
