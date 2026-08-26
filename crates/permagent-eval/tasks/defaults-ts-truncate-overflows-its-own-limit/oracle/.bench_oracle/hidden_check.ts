import assert from "node:assert/strict";
import { truncate } from "./src/utils/string.ts";

assert.equal(truncate("Hello, World!", 8), "Hello...", "truncate should reserve room for the ellipsis");
assert.equal(truncate("Hi", 5), "Hi", "strings within the limit should be untouched");
assert.equal(truncate("abcdefgh", 8), "abcdefgh", "exact-length strings should be untouched");
assert.equal(truncate("Supercalifragilistic", 11), "Supercal...", "total length including ellipsis must equal maxLength");
assert.equal(truncate("Supercalifragilistic", 11).length, 11);

console.log("all assertions passed");
