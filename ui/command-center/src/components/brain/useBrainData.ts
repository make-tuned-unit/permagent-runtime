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
export interface GraphMemory { id: string; key?: string | null; text: string; description: string | null; ent: string[]; age: number; weight: number; timestamp: string }
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

  const fetchGraph = useCallback(async () => {
    const seq = ++seqRef.current;
    try {
      const q = queryRef.current.trim();
      const endpoint = q
        ? `/api/brain/graph?q=${encodeURIComponent(q)}`
        : '/api/brain/graph';
      const result = await apiFetch<BrainGraph>(endpoint);
      if (seq !== seqRef.current) return;
      if (!result || !Array.isArray(result.entities) || !Array.isArray(result.memories)) {
        throw new Error('Invalid Brain graph response');
      }
      setData(result);
      setError(null);
    } catch (e) {
      if (seq !== seqRef.current) return;
      // Only surface the error when we have nothing to show — a failed poll
      // while data is already on screen keeps the stale graph, not an alarm.
      setError(e instanceof Error ? e.message : 'Could not reach the Brain');
    }
    setLoading(false);
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
