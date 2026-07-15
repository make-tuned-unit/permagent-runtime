// Deterministic grader for the 2048 row-merge task. Node only, no dependencies.
import fs from "node:fs";
import { pathToFileURL } from "node:url";

function fail(msg) {
  console.error("FAIL: " + msg);
  process.exit(1);
}

if (!fs.existsSync("merge.mjs")) fail("merge.mjs not found");
let mod;
try {
  mod = await import(pathToFileURL(process.cwd() + "/merge.mjs").href);
} catch (e) {
  fail("importing merge.mjs threw: " + (e && e.message));
}
const mergeRow = mod.mergeRow;
if (typeof mergeRow !== "function") fail("merge.mjs must export function mergeRow(row)");

if (!fs.existsSync("index.html")) fail("index.html not found");
const html = fs.readFileSync("index.html", "utf8");
if (!/merge\.mjs/.test(html)) fail("index.html must import ./merge.mjs");
if (!/type\s*=\s*["']module["']/i.test(html)) fail("index.html must use a module script");

const cases = [
  [[2, 2, 0, 0], [4, 0, 0, 0]],
  [[2, 0, 2, 0], [4, 0, 0, 0]],
  [[2, 2, 2, 0], [4, 2, 0, 0]],
  [[2, 2, 2, 2], [4, 4, 0, 0]],
  [[8, 0, 4, 4], [8, 8, 0, 0]],
  [[4, 4, 8, 0], [8, 8, 0, 0]],
  [[2, 0, 0, 0], [2, 0, 0, 0]],
  [[0, 0, 0, 0], [0, 0, 0, 0]],
  [[0, 0, 2, 2], [4, 0, 0, 0]],
  [[2, 2, 4, 4], [4, 8, 0, 0]],
];
for (const [input, expected] of cases) {
  const frozen = input.slice();
  let got;
  try {
    got = mergeRow(input.slice());
  } catch (e) {
    fail("mergeRow(" + JSON.stringify(frozen) + ") threw: " + (e && e.message));
  }
  if (!Array.isArray(got) || got.length !== 4 || got.some((v, i) => v !== expected[i])) {
    fail(
      "mergeRow(" +
        JSON.stringify(frozen) +
        ") = " +
        JSON.stringify(got) +
        " but expected " +
        JSON.stringify(expected)
    );
  }
}

console.log("ok: 2048 row-merge verified");
process.exit(0);
