/**
 * Colors + labels for typed graph edges.
 *
 * UX benchmark research §5 (Obsidian vs. Logseq): an untyped link graph "is a
 * map of your linking behavior, not a knowledge graph of typed entities and
 * relationships" — a relationship's line should say what it IS, not just that
 * two nodes are connected. Every `entity` edge (a person→project / person→
 * person line from the graph's triples) carries a `predicate` from the
 * backend (e.g. "works_at", "member_of"); this module turns that string into
 * a stable color and a human label, so two different relationship kinds never
 * render as the same generic purple line. `self` and `memory` edges keep
 * their own fixed colors — they are not relationship triples, they are the
 * scene's own "you" and "recalled from" links.
 *
 * Pure: no three.js, so the hashing/labeling is testable without WebGL.
 */

export type RGB = [number, number, number];

/**
 * One bucket per palette slot, chosen to read distinctly against the scene's
 * dark background and to stay clear of NODE_COLORS/EDGE_COLORS' other hues.
 * Index 0 matches the old flat "entity" purple, so an ungraphed/unknown
 * predicate looks exactly as it always has.
 */
const PREDICATE_PALETTE: RGB[] = [
  [0.66, 0.34, 0.97], // violet — default bucket
  [1.00, 0.55, 0.30], // amber
  [0.30, 0.80, 0.55], // green
  [1.00, 0.42, 0.62], // pink
  [0.40, 0.70, 1.00], // sky
  [0.95, 0.85, 0.30], // yellow
];

/** A tiny stable string hash (FNV-1a-ish) — same predicate always maps to the
 *  same color, across a session and across reloads. */
function hash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (Math.imul(h, 31) + s.charCodeAt(i)) >>> 0;
  return h;
}

/** Color for an entity edge's relationship predicate. No predicate (or an
 *  empty one) falls back to the same default the flat color used to be. */
export function predicateColor(predicate: string | null | undefined): RGB {
  const p = (predicate ?? '').trim();
  if (!p) return PREDICATE_PALETTE[0];
  return PREDICATE_PALETTE[hash(p) % PREDICATE_PALETTE.length];
}

/** "works_at" → "works at" — never a raw snake_case identifier in a tooltip
 *  or legend (R3: no internal identifiers in user copy). */
export function predicateLabel(predicate: string | null | undefined): string {
  const p = (predicate ?? '').trim();
  if (!p) return 'connected';
  return p.replace(/_/g, ' ');
}
