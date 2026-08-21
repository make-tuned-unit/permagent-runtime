import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { brandCopy } from "./brand.js";

describe("brandCopy", () => {
  it("rewrites Goose product copy to Permagent", () => {
    assert.equal(brandCopy("Welcome to goose"), "Welcome to permagent");
    assert.equal(brandCopy("Welcome to Goose"), "Welcome to Permagent");
    assert.equal(
      brandCopy("Run `goose configure` in your terminal"),
      "Run `permagent configure` in your terminal",
    );
  });

  it("leaves config keys alone", () => {
    assert.equal(brandCopy("GOOSE_PROVIDER"), "GOOSE_PROVIDER");
  });

  it("rewrites .goose in paths shown in the UI", () => {
    assert.equal(
      brandCopy("/Users/j/.goose/config.yaml"),
      "/Users/j/.permagent/config.yaml",
    );
  });
});
