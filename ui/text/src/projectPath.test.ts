import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { formatHomePath, projectFolderName } from "./projectPath.js";

describe("formatHomePath", () => {
  it("replaces the home prefix with ~", () => {
    assert.equal(
      formatHomePath("/Users/j/Documents/dev/foo", "/Users/j"),
      "~/Documents/dev/foo",
    );
  });

  it("returns ~ for the home directory itself", () => {
    assert.equal(formatHomePath("/Users/j", "/Users/j"), "~");
  });

  it("leaves paths outside home unchanged", () => {
    assert.equal(formatHomePath("/tmp/work", "/Users/j"), "/tmp/work");
  });
});

describe("projectFolderName", () => {
  it("uses the last path segment", () => {
    assert.equal(
      projectFolderName("/Users/j/Documents/dev/permagent-runtime"),
      "permagent-runtime",
    );
  });
});
