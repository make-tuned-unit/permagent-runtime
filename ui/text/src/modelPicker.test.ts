import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  filterPickerModels,
  modelsFromConfiguredProviders,
  resolveModelSelection,
} from "./modelPicker.js";

const providers = [
  {
    name: "anthropic",
    displayName: "Anthropic",
    isConfigured: true,
    defaultModel: "claude-haiku-4-5",
    knownModels: [{ name: "claude-haiku-4-5" }, { name: "claude-sonnet-4-5" }],
  },
  {
    name: "qwen38_split",
    displayName: "Qwen3.8-27B (split)",
    isConfigured: true,
    defaultModel: "qwen3.8-27b",
    knownModels: [{ name: "qwen3.8-27b" }],
  },
  {
    name: "openai",
    displayName: "OpenAI",
    isConfigured: false,
    defaultModel: "gpt-5.4-mini",
    knownModels: [{ name: "gpt-5.4-mini" }],
  },
];

describe("modelsFromConfiguredProviders", () => {
  it("includes the local split and skips unconfigured providers", () => {
    const models = modelsFromConfiguredProviders(providers);
    assert.deepEqual(
      models.map((m) => `${m.providerName}/${m.model}`).sort(),
      [
        "anthropic/claude-haiku-4-5",
        "anthropic/claude-sonnet-4-5",
        "qwen38_split/qwen3.8-27b",
      ],
    );
  });
});

describe("filterPickerModels", () => {
  it("finds the split by model id, provider id, or display name", () => {
    const models = modelsFromConfiguredProviders(providers);
    assert.equal(filterPickerModels(models, "qwen3.8").length, 1);
    assert.equal(filterPickerModels(models, "qwen38_split").length, 1);
    assert.equal(filterPickerModels(models, "27B").length, 1);
  });
});

describe("resolveModelSelection", () => {
  const models = modelsFromConfiguredProviders(providers);

  it("resolves a bare qwen3.8-27b id to the split provider", () => {
    const hit = resolveModelSelection(models, "qwen3.8-27b");
    assert.ok(!("error" in hit));
    assert.equal(hit.providerName, "qwen38_split");
    assert.equal(hit.model, "qwen3.8-27b");
  });

  it("resolves provider/model", () => {
    const hit = resolveModelSelection(models, "qwen38_split/qwen3.8-27b");
    assert.ok(!("error" in hit));
    assert.equal(hit.providerName, "qwen38_split");
  });

  it("rejects an unknown id", () => {
    const hit = resolveModelSelection(models, "nope");
    assert.ok("error" in hit);
  });
});
