import { useState, useEffect, useRef } from 'react';

export interface GraphSelf { name: string; id: string }
export interface GraphEntity { id: string; type: string; name: string; note: string }
export interface GraphMemory { id: string; text: string; ent: string[]; age: number; weight: number; timestamp: string }
export interface BrainGraph { self: GraphSelf; entities: GraphEntity[]; memories: GraphMemory[] }

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const API = (import.meta.env.VITE_DAEMON_URL as string | undefined) ||
  (import.meta.env.VITE_API_BASE_URL as string | undefined) ||
  (isTauri ? 'http://127.0.0.1:3001' : '');

export function useBrainData() {
  const [data, setData] = useState<BrainGraph | null>(null);
  const [loading, setLoading] = useState(true);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();

  const fetchGraph = async () => {
    try {
      const res = await fetch(`${API}/api/brain/graph`);
      if (res.ok) setData(await res.json());
    } catch { /* ignore */ }
    setLoading(false);
  };

  useEffect(() => {
    fetchGraph();
    intervalRef.current = setInterval(fetchGraph, 60_000);
    return () => clearInterval(intervalRef.current);
  }, []);

  return { data, loading, refresh: fetchGraph };
}
