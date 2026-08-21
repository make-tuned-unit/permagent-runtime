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
});
