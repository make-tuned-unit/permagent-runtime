import assert from "node:assert/strict";
import { rankResults } from "./src/features/search/rank.ts";

const items = [
  { id: "a", title: "A", score: 10 },
  { id: "b", title: "B", score: 50, pinned: true },
  { id: "c", title: "C", score: 5, pinned: true },
  { id: "d", title: "D", score: 20 },
];

const ranked = rankResults(items).map((i) => i.id);
assert.deepEqual(ranked, ["b", "c", "d", "a"], "pinned items must sort before unpinned, regardless of score");

// No pinned items: falls back to plain score-descending order.
const plain = rankResults([
  { id: "x", title: "X", score: 1 },
  { id: "y", title: "Y", score: 9 },
]).map((i) => i.id);
assert.deepEqual(plain, ["y", "x"]);

console.log("all assertions passed");
