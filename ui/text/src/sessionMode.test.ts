import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  formatTokenCount,
  nextSessionMode,
  parseSessionMode,
  resolveAcpModeId,
} from "./sessionMode.js";

describe("parseSessionMode", () => {
  it("accepts aliases", () => {
    assert.equal(parseSessionMode("auto"), "auto");
    assert.equal(parseSessionMode("yolo"), "auto");
    assert.equal(parseSessionMode("ask"), "approve");
    assert.equal(parseSessionMode("plan"), "chat");
    assert.equal(parseSessionMode("nope"), null);
  });
});

describe("nextSessionMode", () => {
  it("cycles auto → ask → chat → auto", () => {
    assert.equal(nextSessionMode("auto"), "approve");
    assert.equal(nextSessionMode("approve"), "chat");
    assert.equal(nextSessionMode("chat"), "auto");
  });
});

describe("resolveAcpModeId", () => {
  it("falls back to the raw id", () => {
    assert.equal(resolveAcpModeId("auto", undefined), "auto");
  });

  it("matches advertised ids", () => {
    assert.equal(
      resolveAcpModeId("chat", [{ id: "plan" }, { id: "yolo" }]),
      "plan",
    );
    assert.equal(
      resolveAcpModeId("auto", [{ id: "yolo" }, { id: "default" }]),
      "yolo",
    );
  });
});

describe("formatTokenCount", () => {
  it("uses a tilde estimate", () => {
    assert.equal(formatTokenCount(12), "~12");
    assert.equal(formatTokenCount(1500), "~1.5k");
  });
});
