import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  MOBIUS_H,
  MOBIUS_INTRO_FRAMES,
  getMobiusIntroFrame,
} from "./mobius.js";

describe("möbius intro", () => {
  it("has a full intro of 5-line frames", () => {
    assert.equal(getMobiusIntroFrame(0).length, MOBIUS_H);
    assert.equal(getMobiusIntroFrame(MOBIUS_INTRO_FRAMES - 1).length, MOBIUS_H);
  });

  it("settles on a ribbon that includes braille cells", () => {
    const settled = getMobiusIntroFrame(MOBIUS_INTRO_FRAMES - 1);
    const text = settled
      .map((runs) => runs.map((r) => r.text).join(""))
      .join("");
    assert.match(text, /[\u2800-\u28FF]/);
  });

  it("settles on the Permagent brand cyan, not a white/gray fallback", () => {
    const settled = getMobiusIntroFrame(MOBIUS_INTRO_FRAMES - 1);
    const colors = new Set(
      settled.flatMap((runs) => runs.map((r) => r.color.toUpperCase())),
    );
    assert.ok(colors.has("#00D5FF"), `brand cyan missing: ${[...colors]}`);
    assert.ok(
      ![...colors].every((c) => c === "#FFFFFF" || c === "#444444"),
      "ribbon collapsed to white/wait",
    );
  });
});
