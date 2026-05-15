import { useState, useEffect, useRef, useCallback } from 'react';

export interface GraphSelf { name: string; id: string }
export interface GraphEntity { id: string; type: string; name: string; note: string }
export interface GraphMemory { id: string; key?: string | null; text: string; description: string | null; ent: string[]; age: number; weight: number; timestamp: string }
export interface BrainGraph { self: GraphSelf; entities: GraphEntity[]; memories: GraphMemory[] }

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const API = (import.meta.env.VITE_DAEMON_URL as string | undefined) ||
  (import.meta.env.VITE_API_BASE_URL as string | undefined) ||
  (isTauri ? 'http://127.0.0.1:3001' : '');

export function useBrainData(searchQuery = '') {
  const [data, setData] = useState<BrainGraph | null>(null);
  const [loading, setLoading] = useState(true);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();
  const queryRef = useRef(searchQuery);
  queryRef.current = searchQuery;

  const fetchGraph = useCallback(async () => {
    try {
      const q = queryRef.current.trim();
      const url = q
        ? `${API}/api/brain/graph?q=${encodeURIComponent(q)}`
        : `${API}/api/brain/graph`;
      const res = await fetch(url);
      if (res.ok) setData(await res.json());
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
