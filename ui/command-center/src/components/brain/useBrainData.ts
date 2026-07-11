import { useState, useEffect, useRef, useCallback } from 'react';
import { apiFetch } from '../../lib/api';

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
  const intervalRef = useRef<ReturnType<typeof setInterval>>();
  const queryRef = useRef(searchQuery);
  queryRef.current = searchQuery;

  const fetchGraph = useCallback(async () => {
    try {
      const q = queryRef.current.trim();
      const endpoint = q
        ? `/api/brain/graph?q=${encodeURIComponent(q)}`
        : '/api/brain/graph';
      const result = await apiFetch<BrainGraph>(endpoint);
      setData(result);
    } catch { /* ignore */ }
    setLoading(false);
  }, []);

  useEffect(() => {
    fetchGraph();
    intervalRef.current = setInterval(fetchGraph, 60_000);
    return () => clearInterval(intervalRef.current);
  }, [fetchGraph]);

  // Re-fetch when search query changes
  useEffect(() => {
    fetchGraph();
  }, [searchQuery, fetchGraph]);

  return { data, loading, refresh: fetchGraph };
}
