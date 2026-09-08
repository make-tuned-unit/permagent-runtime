import assert from "node:assert/strict";

import { quoteShipping } from "../src/shipping/quote.ts";
import { buildCheckoutSummary } from "../src/checkout/checkout.ts";
import { estimateCartShipping } from "../src/cart/cartSummary.ts";
import { computeManualQuote } from "../src/admin/manualQuote.ts";
import { handleQuoteRequest } from "../src/api/quoteHandler.ts";

// quoteShipping itself must now accept a package type and apply a 25%
// surcharge for fragile packages.
assert.equal(quoteShipping(10, "domestic", "standard"), 2000);
assert.equal(quoteShipping(10, "domestic", "fragile"), 2500);
assert.equal(quoteShipping(10, "international", "fragile"), Math.round(5500 * 1.25));

const standardOrder = { id: "o1", weightKg: 10, zone: "domestic" as const, packageType: "standard" as const };
const fragileOrder = { id: "o2", weightKg: 10, zone: "domestic" as const, packageType: "fragile" as const };

// checkout must thread the package type through.
const std = buildCheckoutSummary(standardOrder);
const frag = buildCheckoutSummary(fragileOrder);
assert.equal(std.shippingCents, 2000, "standard checkout shipping unaffected");
assert.equal(
  frag.shippingCents,
  Math.round(std.shippingCents * 1.25),
  "checkout must apply the fragile surcharge",
);

// cart preview must thread the package type through, per-order.
assert.equal(estimateCartShipping([fragileOrder]), 2500, "cart summary must apply the fragile surcharge");
assert.equal(estimateCartShipping([standardOrder, fragileOrder]), 2000 + 2500);

// admin manual-quote tool must support package type.
assert.equal(computeManualQuote(10, "domestic", "standard"), 2000);
assert.equal(computeManualQuote(10, "domestic", "fragile"), 2500, "admin tool must support package type");

// API handler must thread the package type through.
assert.equal(handleQuoteRequest(standardOrder).shippingCents, 2000);
assert.equal(handleQuoteRequest(fragileOrder).shippingCents, 2500, "API handler must apply the fragile surcharge");

console.log("all assertions passed");
