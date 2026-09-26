import { useState, useEffect, useRef, useCallback } from 'react';
import { apiFetch } from '../../lib/api';
import { subscribeWorldEvents } from '../world/shared/worldEvents';

export interface GraphSelf { name: string; id: string }
export interface GraphEntityField {
  field_name: string;
  value: string;
  /** Provenance: 'manual' | 'enriched'. Manual is never clobbered by enrichment. */
  source: string;
  source_url?: string | null;
  updated_at: string;
}
export interface GraphEntity { id: string; type: string; name: string; note: string; fields?: GraphEntityField[] }
/** Entity→entity connection (graph triple) — person→project / person→person lines (#495 slice 3). */
export interface GraphEdge { from: string; to: string; predicate: string }
export interface GraphMemory {
  id: string;
  key?: string | null;
  text: string;
  description: string | null;
  ent: string[];
  age: number;
  weight: number;
  timestamp: string;
  layer?: string | null;
  why?: string | null;
  /** The `RememberOpts.source` tag this memory was fed with ("chat",
   *  "permagent.card", "permagent.goal", …) — `null`/absent for a memory
   *  written before the source column existed, or for a search-bar hit
   *  (`/api/brain/search` doesn't carry it). The UI shows "source unknown"
   *  rather than guessing (R5 "a memory shows its source"). */
  source?: string | null;
  /** When this memory was last reinforced (Spectral's Memify feedback loop) —
   *  the closest durable signal to "last referenced" Spectral carries.
   *  `null`/absent means never reinforced, or unavailable for this row. */
  last_reinforced_at?: string | null;
}
export interface BrainGraph { self: GraphSelf; entities: GraphEntity[]; edges?: GraphEdge[]; memories: GraphMemory[] }

export function useBrainData(searchQuery = '') {
  const [data, setData] = useState<BrainGraph | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const queryRef = useRef(searchQuery);
  queryRef.current = searchQuery;
  // Monotonic request sequence — a response only lands if no newer request has
  // started since, so a slow stale reply can never clobber a fresher graph.
  const seqRef = useRef(0);

  // Resolves `true` only when this call actually landed a graph, so the
  // "Try again" button that awaits it cannot flash a success tick over an
  // error message that is still on screen.
  const fetchGraph = useCallback(async () => {
    const seq = ++seqRef.current;
    let ok = false;
    try {
      const q = queryRef.current.trim();
      const endpoint = q
        ? `/api/brain/graph?q=${encodeURIComponent(q)}`
        : '/api/brain/graph';
      const result = await apiFetch<BrainGraph>(endpoint);
      if (seq !== seqRef.current) return false;
      if (!result || !Array.isArray(result.entities) || !Array.isArray(result.memories)) {
        throw new Error('Invalid Brain graph response');
      }
      setData(result);
      setError(null);
      ok = true;
    } catch (e) {
      if (seq !== seqRef.current) return false;
      // Only surface the error when we have nothing to show — a failed poll
      // while data is already on screen keeps the stale graph, not an alarm.
      setError(e instanceof Error ? e.message : 'Could not reach the Brain');
    }
    setLoading(false);
    return ok;
  }, []);

  // Single query-keyed effect: fetch immediately on mount and whenever the
  // query changes, and poll on one interval. (Previously a mount effect and a
  // query effect both fired on mount — a double-fetch race.)
  useEffect(() => {
    fetchGraph();
    const interval = setInterval(fetchGraph, 60_000);
    return () => clearInterval(interval);
  }, [searchQuery, fetchGraph]);

  // Live growth (#24): a new memory or entity link lands → refetch soon, so
  // the graph visibly grows while the user works instead of waiting out the
  // 60s poll. Debounced — a burst of ingestion collapses to one refetch.
  useEffect(() => {
    let debounce: ReturnType<typeof setTimeout> | undefined;
    const unsubscribe = subscribeWorldEvents((evt) => {
      if (evt.replayed) return;
      if (
        evt.type === 'memory_added' ||
        evt.type === 'entity_added' ||
        evt.type === 'entity_updated'
      ) {
        clearTimeout(debounce);
        debounce = setTimeout(fetchGraph, 1200);
      }
    });
    return () => {
      clearTimeout(debounce);
      unsubscribe();
    };
  }, [fetchGraph]);

  return { data, loading, error, refresh: fetchGraph };
}
