/**
 * Which way the Brain opens (J12).
 *
 * List by default. The 3D force-graph is the most internals-shaped surface in
 * the app — it has no legend, three entity types share one glyph, and its
 * interaction model is undiscoverable — while List has real dates, real search
 * highlighting and nothing to learn. Making the graph the front door meant the
 * first thing a non-technical user met was the hardest thing to read.
 *
 * Graph is a toggle away and the choice sticks, so someone who prefers it says
 * so once. Only a DELIBERATE toggle is remembered: search flips to List on its
 * own while a query is live and flips back when it clears, and neither of those
 * is the user telling us anything.
 */

export type BrainViewMode = 'graph' | 'list';

export const VIEW_MODE_KEY = 'brain:view-mode';

const DEFAULT_MODE: BrainViewMode = 'list';

/** The remembered choice, or List. Storage may be unavailable (a private
 *  window, blocked site data) — that is not a reason to fail to open. */
export function readViewMode(): BrainViewMode {
  try {
    const stored = localStorage.getItem(VIEW_MODE_KEY);
    return stored === 'graph' || stored === 'list' ? stored : DEFAULT_MODE;
  } catch {
    return DEFAULT_MODE;
  }
}

/** Call ONLY from the toggle the user pressed. */
export function rememberViewMode(mode: BrainViewMode): void {
  try {
    localStorage.setItem(VIEW_MODE_KEY, mode);
  } catch {
    // A preference that could not be saved is a preference that does not
    // persist — never a failure the user has to see.
  }
}
