import { quoteShipping } from "../shipping/quote.ts";
import type { Order } from "../types.ts";

export interface QuoteResponse {
  shippingCents: number;
}

/** The public API handler used by the storefront's quote endpoint. */
export function handleQuoteRequest(order: Order): QuoteResponse {
  return { shippingCents: quoteShipping(order.weightKg, order.zone) };
}
