import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  enableAutonomous,
  idleAutonomous,
  parseAutonomousArgs,
  shouldAutoContinue,
} from "./autonomous.js";

describe("parseAutonomousArgs", () => {
  it("parses on/off/status/gate", () => {
    assert.deepEqual(parseAutonomousArgs(""), { action: "status" });
    assert.deepEqual(parseAutonomousArgs("on"), { action: "on" });
    assert.deepEqual(parseAutonomousArgs("on 20"), {
      action: "on",
      maxTurns: 20,
    });
    assert.deepEqual(parseAutonomousArgs("on 8 gate npm test"), {
      action: "on",
      maxTurns: 8,
      gate: "npm test",
    });
    assert.deepEqual(parseAutonomousArgs("off"), { action: "off" });
    assert.deepEqual(parseAutonomousArgs("gate cargo test"), {
      action: "gate",
      command: "cargo test",
    });
    assert.equal(parseAutonomousArgs("wat").action, "error");
  });
});

describe("shouldAutoContinue", () => {
  it("continues only on a clean end_turn", () => {
    const on = enableAutonomous(idleAutonomous(), 3);
    assert.equal(
      shouldAutoContinue(on, {
        stopReason: "end_turn",
        queueEmpty: true,
        cancelled: false,
      }).continue,
      true,
    );
    assert.equal(
      shouldAutoContinue(on, {
        stopReason: "end_turn",
        queueEmpty: true,
        cancelled: true,
      }).continue,
      false,
    );
    assert.equal(
      shouldAutoContinue(on, {
        stopReason: "end_turn",
        queueEmpty: false,
        cancelled: false,
      }).continue,
      false,
    );
  });

  it("stops at the turn cap", () => {
    const on = { ...enableAutonomous(idleAutonomous(), 2), turnsUsed: 2 };
    const r = shouldAutoContinue(on, {
      stopReason: "end_turn",
      queueEmpty: true,
      cancelled: false,
    });
    assert.equal(r.continue, false);
    if (!r.continue) assert.match(r.reason ?? "", /hit 2/);
  });
});
