/**
 * Brain memory focus — the cross-surface "View in Brain" seam contract plus its
 * pure resolution logic.
 *
 * Product surfaces that WRITE into the Brain (the Projects Memories panel, the
 * Notes panel, the Codebase index panel, …) close the loop by deep-linking a
 * specific memory back into the Brain view. They call the store's
 * `focusBrainMemory(target)`, which stashes the target and navigates to the
 * Brain; BrainView consumes it and opens that memory's side-panel.
 *
 * Resolution is graph-preferred with a preview fallback, because a freshly
 * written memory is often NOT in the Brain graph yet: the graph's default view
 * is capped (MAX_MEMORIES) and, critically, filters `description IS NOT NULL`
 * (see the brain_graph handler), while product writes are description-less until
 * the Librarian enriches them. So callers pass an optional `preview` (the
 * content they already hold) that BrainView renders when the live graph can't
 * yet resolve the id/key — the real graph memory always wins when present.
 */

import { freshnessOf, parseTimestamp } from '../../hooks/useFreshness';
import type { GraphMemory } from './useBrainData';
import type { ProjectMemory } from '../projects/types';

/** A memory the caller already holds enough of to render without a graph hit. */
export interface BrainMemoryPreview {
  text: string;
  description?: string | null;
  /** Spectral signal_score (clamped to 0..1 for display). */
  weight?: number | null;
  /** Creation/association time; drives the recency bucket. */
  timestamp?: string | null;
}

/**
 * What a surface wants the Brain to focus. `id` is the Spectral memory id; `key`
 * is the stable memory key (`note:…`, `code:…`). At least one of id/key should
 * be set. `preview` is the caller's already-held content, rendered only as a
 * fallback when the live graph can't resolve the memory.
 */
export interface BrainMemoryTarget {
  id?: string | null;
  key?: string | null;
  preview?: BrainMemoryPreview | null;
}

export type FocusResolution =
  | { kind: 'graph'; memory: GraphMemory }
  | { kind: 'preview'; memory: GraphMemory }
  | { kind: 'none' };

const NINETY_DAYS_SECS = 90 * 24 * 3600;

function clamp01(n: number): number {
  if (Number.isNaN(n)) return 0;
  return Math.min(1, Math.max(0, n));
}

/**
 * Parse a backend timestamp — ISO-8601, or SQLite's "YYYY-MM-DD HH:MM:SS" (UTC,
 * no zone) — to epoch ms, or null if unparseable. Mirrors NotesPanel's
 * relativeTime: a bare "date time" string is treated as UTC.
 */
export function parseBrainTimestamp(ts: string): number | null {
  return parseTimestamp(ts);
}

/**
 * Normalized 0..1 age over a 90-day window (mirrors the brain_graph handler) so
 * a synthesized memory sits in the scene where a graph one would.
 * Missing/unparseable timestamps fall back to 0.5, matching the backend.
 *
 * This is a SCENE COORDINATE, not a date: it drives the fresh→stale colour lerp
 * and the time-range slider, both of which need a bounded number. It is clamped,
 * so every memory older than 90 days shares the value 1 — never render a label
 * from it. `formatMemoryAge` below is what a human reads.
 */
export function ageFromTimestamp(ts: string | null | undefined, now = Date.now()): number {
  if (!ts) return 0.5;
  const then = parseBrainTimestamp(ts);
  if (then == null) return 0.5;
  const secs = Math.max(0, (now - then) / 1000);
  return clamp01(secs / NINETY_DAYS_SECS);
}

/** Past this, a memory is old enough that its age has to be visible rather than
 *  merely present — the same threshold the scene's colour ramp tops out at. */
export const MEMORY_STALE_AFTER_DAYS = 90;

/** How a memory's age reads to a person, plus whether it is old enough that the
 *  reading itself must carry weight. */
export interface MemoryAge { label: string; stale: boolean }

/**
 * A memory's real age, at whatever magnitude it happens to be.
 *
 * Recency used to be rendered from `ageFromTimestamp`'s clamped scalar, which
 * meant 91 days and 3 years were the same number and therefore the same words
 * ("~year"). Anything past the window looked equally recent — the exact class of
 * claim the liveness rule exists to forbid, on the surface whose whole subject
 * is when something was learned. Buckets stay coarse where coarse is honest
 * (nobody needs "17 days ago"), but they never stop counting.
 */
export function formatMemoryAge(ts: string | null | undefined, now = Date.now()): MemoryAge {
  const { label, stale } = freshnessOf(
    ts,
    {
      // A memory is a dated thing, not a polled one: it reads in days and
      // coarser, so a note written this morning is "today" and not "4h ago".
      granularity: 'calendar',
      // Just past the window the scene's colour ramp tops out at, so the label
      // and the scene agree about when a memory has gone quiet. `>` not `>=`,
      // as before: day 90 is still inside the window.
      staleAfterMs: (MEMORY_STALE_AFTER_DAYS + 1) * 86_400_000,
      unknownLabel: 'date unknown',
    },
    now,
  );
  return { label, stale };
}

/** ProjectMemory (the `/api/projects/:id/memories` wire shape) → focus preview. */
export function projectMemoryPreview(m: ProjectMemory): BrainMemoryPreview {
  return {
    text: m.content,
    description: m.description,
    weight: m.signal_score,
    timestamp: m.created_at,
  };
}

/** Synthesize a GraphMemory from a target's preview (the fallback render). */
export function previewToGraphMemory(target: BrainMemoryTarget, now = Date.now()): GraphMemory {
  const p = target.preview ?? { text: '' };
  return {
    id: target.id ?? '',
    key: target.key ?? null,
    text: p.text ?? '',
    description: p.description ?? null,
    ent: [],
    age: ageFromTimestamp(p.timestamp, now),
    weight: clamp01(p.weight ?? 0),
    timestamp: p.timestamp ?? new Date(now).toISOString(),
  };
}

/**
 * Resolve a focus target against the live graph memories — graph-preferred with
 * a preview fallback. Returns 'none' when neither a graph hit nor a preview is
 * available; the caller decides whether to wait for the graph to load or give up.
 */
export function resolveFocusedMemory(
  target: BrainMemoryTarget,
  graphMemories: GraphMemory[],
  now = Date.now(),
): FocusResolution {
  const byId = target.id ? graphMemories.find(m => m.id === target.id) : undefined;
  const hit = byId ?? (target.key ? graphMemories.find(m => m.key === target.key) : undefined);
  if (hit) return { kind: 'graph', memory: hit };
  if (target.preview) return { kind: 'preview', memory: previewToGraphMemory(target, now) };
  return { kind: 'none' };
}

/**
 * Human title for a memory: description first-sentence → humanized key →
 * content preview. The single source of truth shared by the Brain list, the 3D
 * scene selection, and the cross-surface focus seam so a memory reads the same
 * everywhere it surfaces.
 */
export function deriveMemoryTitle(mem: GraphMemory): string {
  // Description-first: use first sentence.
  if (mem.description) {
    const firstSentence = mem.description.split(/\.\s/)[0];
    if (firstSentence.length > 0 && firstSentence.length <= 80) {
      return firstSentence.endsWith('.') ? firstSentence : firstSentence + '.';
    }
    if (firstSentence.length > 80) {
      return firstSentence.slice(0, 77) + '...';
    }
  }
  // Key fallback: humanize.
  if (mem.key) {
    return humanizeMemoryKey(mem.key);
  }
  // Last resort: content preview.
  return mem.text.slice(0, 60) + (mem.text.length > 60 ? '...' : '');
}

/**
 * Humanize a memory key (`daily-standup_2024...` → "Daily Standup") — drops hex
 * hashes and long digit runs, title-cases the rest.
 */
export function humanizeMemoryKey(key: string): string {
  // Split on : _ -
  const parts = key.split(/[:_\-]+/);
  // Drop hex hashes (>6 hex chars), timestamps (all-digit >8 chars).
  const clean = parts.filter(p => {
    if (p.length > 8 && /^\d+$/.test(p)) return false;
    if (p.length > 6 && /^[0-9a-f]+$/i.test(p)) return false;
    return p.length > 0;
  });
  if (clean.length === 0) return key.slice(0, 40);
  // Title case.
  return clean.map(w => w.charAt(0).toUpperCase() + w.slice(1).toLowerCase()).join(' ');
}
