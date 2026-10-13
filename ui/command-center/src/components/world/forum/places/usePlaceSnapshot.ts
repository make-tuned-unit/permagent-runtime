/**
 * World W2 — read one place's evidence through the shared W1 loader.
 *
 * One visit per (place, project) change. The loader dedupes, caches and caps
 * requests; this hook adds only latest-request-wins for THIS panel, so a
 * superseded answer never overwrites a newer one. Reads only — the loader
 * cannot send or write.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { WorldSnapshotLoader, PlaceSnapshot } from '../data/snapshotLoader';
import type { WorldPlaceId } from '../data/identity';
import { PLACE_REQUEST_PLAN } from '../data/snapshotLoader';

export interface PlaceSnapshotState {
  snapshot: PlaceSnapshot | null;
  loading: boolean;
  /** The loader itself threw (a programming error, not a daemon failure). */
  failure: string | null;
}

export function usePlaceSnapshot(loader: WorldSnapshotLoader, placeId: WorldPlaceId, projectId: string | null) {
  const [state, setState] = useState<PlaceSnapshotState>({ snapshot: null, loading: true, failure: null });
  const token = useRef(0);
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    const mine = ++token.current;
    setState((s) => ({ ...s, loading: true }));
    loader.loadPlace(placeId, { projectId }).then(
      (snapshot) => { if (mine === token.current) setState({ snapshot, loading: false, failure: null }); },
      (error: unknown) => { if (mine === token.current) setState((s) => ({ ...s, loading: false, failure: error instanceof Error ? error.message : String(error) })); },
    );
    return () => { if (mine === token.current) token.current += 1; };
  }, [loader, placeId, projectId, nonce]);

  /** User-initiated retry: mark this place's endpoints stale, then read again. */
  const retry = useCallback(() => {
    const prefixes = PLACE_REQUEST_PLAN[placeId].map((p) => p.split(/[ ?{]/)[0]);
    loader.invalidate(prefixes);
    setNonce((n) => n + 1);
  }, [loader, placeId]);

  return { ...state, retry };
}
