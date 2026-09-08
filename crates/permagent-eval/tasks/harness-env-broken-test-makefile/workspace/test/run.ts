import assert from "node:assert/strict";

import { add, multiply, average } from "../src/calc.ts";
import { formatCurrency } from "../src/format.ts";
import { median } from "../src/stats.ts";

assert.equal(add(2, 3), 5);
assert.equal(multiply(4, 5), 20);
assert.equal(average([1, 2, 3]), 2);
assert.equal(formatCurrency(1234), "$12.34");
assert.equal(formatCurrency(-50), "-$0.50");
assert.equal(median([5, 1, 3]), 3);
assert.equal(median([5, 1, 3, 7]), 4);

console.log("ALL_TESTS_PASSED_MARKER_9f3a");
