/**
 * The Brain graph's colours, in one place.
 *
 * They live here rather than inside the scene because the key that explains
 * them (`brainLegend.tsx`) has to draw the *same* swatches the scene draws.
 * Three entity kinds — project, tool and organisation — share one cube in the
 * scene and one ■ in the filter row, so colour is the only thing that tells
 * them apart. A key that guessed at those colours would be a second opinion
 * about the same fact, which is how the Forecaster spent weeks claiming a wire
 * it did not have.
 */

/** Entity node colours, by Spectral entity type. */
export const NODE_COLORS: Record<string, number> = {
  person: 0xc8e0ff,
  project: 0xa855f7,
  tool: 0x22d3ee,
  location: 0x4ade80,
  organization: 0xfb923c,
  concept: 0x7bb7ff,
};

/** The fallback the scene uses for a type it has no colour for. */
export const NODE_COLOR_FALLBACK = 0x7bb7ff;

/** A memory's colour is this, fading toward `MEMORY_STALE` as it ages. */
export const MEMORY_FRESH = 0x00d5ff;
export const MEMORY_STALE = 0x4a5468;

export function hex(color: number): string {
  return `#${color.toString(16).padStart(6, '0')}`;
}
