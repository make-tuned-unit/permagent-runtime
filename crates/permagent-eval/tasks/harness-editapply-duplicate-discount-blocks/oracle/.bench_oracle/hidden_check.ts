import assert from "node:assert/strict";

import {
  computeBronzeDiscount,
  computeSilverDiscount,
  computeGoldDiscount,
  computePlatinumDiscount,
  computeDiscount,
} from "../src/pricing.ts";

// Gold must now be 20% off, capped at $50.00.
assert.equal(computeGoldDiscount(10000).discountCents, 2000, "gold tier must be 20% off");
assert.equal(computeGoldDiscount(100000).discountCents, 5000, "gold tier discount must cap at $50");
assert.equal(computeGoldDiscount(500000).discountCents, 5000, "gold tier discount must cap at $50 on large orders");

// The other three tiers -- including silver, which coincidentally shared
// gold's old (wrong) 0.15 percentage -- must be completely unaffected by the
// fix. A blind text-replace of "0.15" -> "0.20" would break silver too.
assert.equal(computeSilverDiscount(10000).discountCents, 1500, "silver tier must remain unchanged at 15%");
assert.equal(computeSilverDiscount(200000).discountCents, 2000, "silver tier cap must remain unchanged at $20");
assert.equal(computeBronzeDiscount(10000).discountCents, 500, "bronze tier must remain unchanged at 5%");
assert.equal(computeBronzeDiscount(10000000).discountCents, 500000, "bronze tier must remain uncapped");
assert.equal(computePlatinumDiscount(10000).discountCents, 3000, "platinum tier must remain unchanged at 30%");
assert.equal(computePlatinumDiscount(1000000).discountCents, 10000, "platinum tier cap must remain unchanged at $100");

assert.equal(computeDiscount("gold", 10000).discountCents, 2000);
assert.equal(computeDiscount("silver", 10000).discountCents, 1500);

console.log("all assertions passed");
