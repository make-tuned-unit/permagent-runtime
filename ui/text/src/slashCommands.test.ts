import assert from "node:assert/strict";
import path from "node:path";
import { describe, it } from "node:test";
import {
  filterSlashCommands,
  isSlashMenuOpen,
  parseSlashInput,
  resolveSlashCommand,
  resolveUserPath,
} from "./slashCommands.js";

describe("isSlashMenuOpen", () => {
  it("opens on / and /model", () => {
    assert.equal(isSlashMenuOpen("/"), true);
    assert.equal(isSlashMenuOpen("/mod"), true);
    assert.equal(isSlashMenuOpen("/model"), true);
  });

  it("closes once arguments start", () => {
    assert.equal(isSlashMenuOpen("/model "), false);
    assert.equal(isSlashMenuOpen("/model opus"), false);
  });

  it("stays closed for normal prompts", () => {
    assert.equal(isSlashMenuOpen(""), false);
    assert.equal(isSlashMenuOpen("hello"), false);
  });
});

describe("parseSlashInput", () => {
  it("splits name and args", () => {
    assert.deepEqual(parseSlashInput("/model"), { name: "model", args: "" });
    assert.deepEqual(parseSlashInput("/model  gpt-4.1"), {
      name: "model",
      args: "gpt-4.1",
    });
  });

  it("ignores multiline input", () => {
    assert.equal(parseSlashInput("/model\nplease"), null);
  });
});

describe("resolveSlashCommand", () => {
  it("resolves aliases", () => {
    assert.equal(resolveSlashCommand("mcp")?.name, "extensions");
    assert.equal(resolveSlashCommand("new")?.name, "clear");
    assert.equal(resolveSlashCommand("q")?.name, "quit");
  });
});

describe("resolveUserPath", () => {
  it("expands ~ and relative paths", () => {
    assert.equal(resolveUserPath("~", "/tmp", "/Users/j"), "/Users/j");
    assert.equal(
      resolveUserPath("~/src", "/tmp", "/Users/j"),
      path.join("/Users/j", "src"),
    );
    assert.equal(
      resolveUserPath("foo", "/tmp/proj", "/Users/j"),
      path.resolve("/tmp/proj", "foo"),
    );
  });
});

describe("filterSlashCommands", () => {
  it("filters by prefix", () => {
    const names = filterSlashCommands("mod").map((c) => c.name);
    assert.deepEqual(names, ["model", "mode"]);
  });

  it("resolves mode aliases", () => {
    assert.equal(resolveSlashCommand("trust")?.name, "mode");
    assert.equal(resolveSlashCommand("auto")?.name, "autonomous");
    assert.equal(resolveSlashCommand("session")?.name, "status");
  });

  it("lists everything for an empty stem", () => {
    assert.ok(filterSlashCommands("").length >= 8);
  });
});
