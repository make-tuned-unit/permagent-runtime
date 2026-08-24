import { useEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { ageFromTimestamp } from './brainMemoryFocus';
import type { GraphMemory } from './useBrainData';

export interface BrainSearchResult {
  source: string;
  id: string;
  preview: string;
  score: number;
  timestamp: string;
  session_id?: string | null;
  layer?: string | null;
  why?: string | null;
}

export interface BrainSearchResponse {
  results: BrainSearchResult[];
  total: number;
  query: string;
  offset: number;
  limit: number;
  fts_count: number;
  spectral_count: number;
  dedup_count: number;
}

export function searchResultToGraphMemory(result: BrainSearchResult): GraphMemory {
  return {
    id: result.id,
    key: result.source === 'chat' ? `chat:${result.session_id ?? result.id}` : null,
    text: result.preview,
    description: null,
    ent: [],
    age: ageFromTimestamp(result.timestamp),
    weight: Math.min(1, Math.max(0, result.score)),
    timestamp: result.timestamp,
    layer: result.layer ?? null,
    why: result.why ?? null,
  };
}

/** Map a ranked search hit to a graph memory node id when the live graph has one. */
export function resolveSearchGraphNode(
  result: BrainSearchResult,
  graphMemories: GraphMemory[],
): string | null {
  if (result.source === 'memory' && !result.id.startsWith('spectral:')) {
    const exact = graphMemories.find(m => m.id === result.id);
    if (exact) return exact.id;
  }

  const preview = result.preview.toLowerCase().trim();
  if (!preview) return null;

  let best: GraphMemory | null = null;
  let bestScore = 0;
  for (const mem of graphMemories) {
    const hay = `${mem.description ?? ''} ${mem.text} ${mem.key ?? ''}`.toLowerCase();
    const needle = preview.slice(0, Math.min(80, preview.length));
    if (!needle || !hay.includes(needle.slice(0, 40))) {
      const textHead = mem.text.toLowerCase().slice(0, 40);
      if (!textHead || !preview.includes(textHead)) continue;
    }
    const score = Math.min(hay.length, preview.length);
    if (score > bestScore) {
      bestScore = score;
      best = mem;
    }
  }
  return best?.id ?? null;
}

export function useBrainSearch(query: string) {
  const [results, setResults] = useState<BrainSearchResult[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seqRef = useRef(0);

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResults(null);
      setError(null);
      setLoading(false);
      return;
    }

    const seq = ++seqRef.current;
    setLoading(true);
    setError(null);

    api.searchBrain({ q })
      .then(res => {
        if (seq !== seqRef.current) return;
        if (!res || !Array.isArray(res.results)) {
          throw new Error('Invalid search response');
        }
        setResults(res.results);
        setError(null);
      })
      .catch(e => {
        if (seq !== seqRef.current) return;
        setResults([]);
        setError(e instanceof Error ? e.message : 'Unknown error');
      })
      .finally(() => {
        if (seq === seqRef.current) setLoading(false);
      });
  }, [query]);

  return { results, loading, error };
}
