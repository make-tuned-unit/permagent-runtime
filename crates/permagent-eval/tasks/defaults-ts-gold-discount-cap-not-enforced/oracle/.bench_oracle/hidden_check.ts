import assert from "node:assert/strict";
import { applyLoyaltyDiscount } from "./src/features/cart/discount.ts";

// Bronze/silver unaffected by the cap.
assert.equal(applyLoyaltyDiscount(10000, "bronze"), 9500);
assert.equal(applyLoyaltyDiscount(10000, "silver"), 9000);

// Gold under the cap: normal 15% applies.
assert.equal(applyLoyaltyDiscount(10000, "gold"), 8500);

// Gold over the cap: discount must be clamped to $50.00 (5000 cents).
assert.equal(applyLoyaltyDiscount(50000, "gold"), 45000, "gold discount must be capped at 5000 cents");
assert.equal(applyLoyaltyDiscount(100000, "gold"), 95000, "gold discount must be capped at 5000 cents");

console.log("all assertions passed");
