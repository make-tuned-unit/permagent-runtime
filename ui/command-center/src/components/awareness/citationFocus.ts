/**
 * Citation → Brain focus mapping. Turns a chat message's memory-citation ref
 * (the "based on N memories" popover) into a {@link BrainMemoryTarget} for the
 * shared `focusBrainMemory` seam (#753) — graph-preferred with a preview
 * fallback, so a cited memory opens in the Brain even when it isn't in the live
 * graph yet (fresh, description-less writes are excluded from the graph until
 * the Librarian enriches them).
 *
 * The refs already carry the Spectral memory id (probed also carries the stable
 * key) and the `content_summary` the marker renders; those become the target's
 * id/key + preview text, and the relevance / signal score the marker already
 * shows becomes the preview's display weight. An empty id/key normalizes to
 * null so resolution falls straight through to the preview rather than matching
 * a blank. Pure + framework-free so it unit-tests without a DOM.
 */

import type { BrainMemoryTarget } from '../brain/brainMemoryFocus';
import type { ProbedMemoryRef, RecalledMemoryRef } from '../../lib/store';

/** Probed citation → focus target (carries the stable key too). */
export function probedFocusTarget(m: ProbedMemoryRef): BrainMemoryTarget {
  return {
    id: m.id || null,
    key: m.key || null,
    preview: { text: m.content_summary, weight: m.relevance },
  };
}

/** Recalled citation → focus target (recalled refs carry no key). */
export function recalledFocusTarget(m: RecalledMemoryRef): BrainMemoryTarget {
  return {
    id: m.id || null,
    preview: { text: m.content_summary, weight: m.signal_score },
  };
}
