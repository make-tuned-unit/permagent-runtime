// Deterministic grader for the tic-tac-toe task. Node only, no dependencies.
// Exit 0 = solved, non-zero = not solved. Run from the finished workspace.
import fs from "node:fs";
import { pathToFileURL } from "node:url";

function fail(msg) {
  console.error("FAIL: " + msg);
  process.exit(1);
}

// 1) The pure logic module must exist and export checkWinner.
if (!fs.existsSync("ttt.mjs")) fail("ttt.mjs not found");
let mod;
try {
  mod = await import(pathToFileURL(process.cwd() + "/ttt.mjs").href);
} catch (e) {
  fail("importing ttt.mjs threw: " + (e && e.message));
}
const checkWinner = mod.checkWinner;
if (typeof checkWinner !== "function") fail("ttt.mjs must export function checkWinner(board)");

// 2) index.html must exist and wire the UI to the tested module.
if (!fs.existsSync("index.html")) fail("index.html not found");
const html = fs.readFileSync("index.html", "utf8");
if (!/ttt\.mjs/.test(html)) fail("index.html must import ./ttt.mjs");
if (!/type\s*=\s*["']module["']/i.test(html)) fail("index.html must use a module script");

// 3) checkWinner must be correct.
const E = "";
const norm = (v) => (v === undefined ? null : v);
const cases = [
  [["X", "X", "X", E, E, E, E, E, E], "X"], // top row
  [[E, E, E, E, E, E, "O", "O", "O"], "O"], // bottom row
  [["O", E, E, "O", E, E, "O", E, E], "O"], // left column
  [["X", E, E, E, "X", E, E, E, "X"], "X"], // main diagonal
  [[E, E, "O", E, "O", E, "O", E, E], "O"], // anti-diagonal
  [["X", "O", "X", "X", "O", "O", "O", "X", "X"], "draw"], // full, no winner
  [["X", "O", E, E, E, E, E, E, E], null], // in progress
  [[E, E, E, E, E, E, E, E, E], null], // empty board
];
for (const [board, expected] of cases) {
  let got;
  try {
    got = checkWinner(board.slice());
  } catch (e) {
    fail("checkWinner(" + JSON.stringify(board) + ") threw: " + (e && e.message));
  }
  if (norm(got) !== expected) {
    fail(
      "checkWinner(" +
        JSON.stringify(board) +
        ") = " +
        JSON.stringify(got) +
        " but expected " +
        JSON.stringify(expected)
    );
  }
}

console.log("ok: tic-tac-toe logic and page verified");
process.exit(0);
