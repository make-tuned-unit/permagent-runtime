/** One row in the `/model` picker — a concrete provider + model id. */
export type PickerModel = {
  providerName: string;
  displayName: string;
  model: string;
};

type KnownModel = { name: string } | string;

type ProviderLike = {
  name: string;
  displayName: string;
  isConfigured: boolean;
  defaultModel?: string;
  knownModels?: KnownModel[];
};

function modelName(entry: KnownModel): string {
  return typeof entry === "string" ? entry : entry.name;
}

/** Models the `/model` picker can switch to, from every configured provider. */
export function modelsFromConfiguredProviders(
  providers: ProviderLike[],
): PickerModel[] {
  const out: PickerModel[] = [];
  for (const p of providers) {
    if (!p.isConfigured) continue;
    const names = new Set<string>();
    for (const m of p.knownModels ?? []) {
      const name = modelName(m);
      if (name) names.add(name);
    }
    if (p.defaultModel) names.add(p.defaultModel);
    for (const model of names) {
      out.push({
        providerName: p.name,
        displayName: p.displayName,
        model,
      });
    }
  }
  return out;
}

export function filterPickerModels(
  models: PickerModel[],
  query: string,
): PickerModel[] {
  const q = query.trim().toLowerCase();
  if (!q) return models;
  return models.filter(
    (m) =>
      m.model.toLowerCase().includes(q) ||
      m.displayName.toLowerCase().includes(q) ||
      m.providerName.toLowerCase().includes(q),
  );
}

/**
 * Resolve `/model <args>` to one picker row.
 * Accepts a bare model id (`qwen3.8-27b`) or `provider/model`.
 */
export function resolveModelSelection(
  models: PickerModel[],
  args: string,
): PickerModel | { error: string } {
  const raw = args.trim();
  if (!raw) return { error: "no model name" };

  const slash = raw.indexOf("/");
  if (slash > 0) {
    const providerName = raw.slice(0, slash);
    const model = raw.slice(slash + 1);
    const hit = models.find(
      (m) =>
        m.providerName.toLowerCase() === providerName.toLowerCase() &&
        m.model.toLowerCase() === model.toLowerCase(),
    );
    if (hit) return hit;
    return { error: `unknown model ${raw}` };
  }

  const q = raw.toLowerCase();
  const exact = models.filter((m) => m.model.toLowerCase() === q);
  if (exact.length === 1) return exact[0]!;
  if (exact.length > 1) {
    return {
      error: `ambiguous: ${exact.map((m) => `${m.providerName}/${m.model}`).join(", ")}`,
    };
  }
  const partial = models.filter(
    (m) =>
      m.model.toLowerCase().includes(q) ||
      m.displayName.toLowerCase().includes(q),
  );
  if (partial.length === 1) return partial[0]!;
  if (partial.length > 1) {
    return {
      error: `ambiguous: ${partial.map((m) => `${m.providerName}/${m.model}`).join(", ")}`,
    };
  }
  return { error: `unknown model ${raw}` };
}
