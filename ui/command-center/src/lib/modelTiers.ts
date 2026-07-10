/**
 * Model recommendation tiers based on available RAM.
 * Easy to update as new models release — this is the single source of truth.
 */

export interface ModelTier {
  /** Minimum RAM in GB to run this model */
  minRamGB: number;
  /** Ollama model tag */
  model: string;
  /** Human-readable label */
  label: string;
  /** Approximate download size for UI display */
  downloadSizeGB: number;
}

export const MODEL_TIERS: ModelTier[] = [
  { minRamGB: 8,  model: 'qwen3:4b',  label: 'Basic',        downloadSizeGB: 2.6 },
  { minRamGB: 16, model: 'qwen3:8b',  label: 'Good balance', downloadSizeGB: 5.2 },
  { minRamGB: 32, model: 'qwen3:14b', label: 'High quality',  downloadSizeGB: 9.0 },
  { minRamGB: 64, model: 'qwen3:32b', label: 'Maximum quality', downloadSizeGB: 20.0 },
];

/** Minimum RAM (GB) to run any local model at all */
export const MIN_RAM_GB = 8;

/**
 * Given total RAM in bytes, return the best model tier for this hardware.
 * Returns null if RAM is below the minimum threshold.
 */
export function recommendModel(totalRamBytes: number): ModelTier | null {
  const ramGB = totalRamBytes / (1024 * 1024 * 1024);
  if (ramGB < MIN_RAM_GB) return null;

  let best: ModelTier = MODEL_TIERS[0];
  for (const tier of MODEL_TIERS) {
    if (ramGB >= tier.minRamGB) best = tier;
  }
  return best;
}

/**
 * Return all model tiers that fit within the given RAM.
 */
export function compatibleModels(totalRamBytes: number): ModelTier[] {
  const ramGB = totalRamBytes / (1024 * 1024 * 1024);
  return MODEL_TIERS.filter(t => ramGB >= t.minRamGB);
}
