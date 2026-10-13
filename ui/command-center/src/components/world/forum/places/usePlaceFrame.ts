/**
 * World W2 — what every place component needs from its host props: theme,
 * compact layout, clock, the place snapshot and a record opener that reports
 * an unreachable destination instead of looking like it worked.
 */
import { useCallback, useMemo, useState } from 'react';
import { openRecordLink, recordLinkKey, type OpenResult, type WorldRecordLink } from '../data/recordLinks';
import type { WorldPlaceId } from '../data/identity';
import type { Section } from '../data/types';
import { snapshotStateLine } from './format';
import type { PlaceHostProps } from './integration';
import { placeColors, placeStyles } from './placeStyles';
import { usePlaceSnapshot } from './usePlaceSnapshot';

/** Below these logical sizes the panel stacks its evidence and collapses secondary slots. */
export const COMPACT_MAX_HEIGHT = 720;
export const COMPACT_MAX_WIDTH = 1100;

export function isCompactViewport(viewport?: { width: number; height: number }): boolean {
  const w = viewport?.width ?? (typeof window !== 'undefined' ? window.innerWidth : COMPACT_MAX_WIDTH);
  const h = viewport?.height ?? (typeof window !== 'undefined' ? window.innerHeight : COMPACT_MAX_HEIGHT + 1);
  return w < COMPACT_MAX_WIDTH || h <= COMPACT_MAX_HEIGHT;
}

export function usePlaceFrame(props: PlaceHostProps, placeId: WorldPlaceId, projectId: string | null) {
  const colors = placeColors();
  const compact = isCompactViewport(props.viewport);
  const styles = useMemo(() => placeStyles(colors, compact), [colors, compact]);
  const now = (props.now ?? Date.now)();
  const snap = usePlaceSnapshot(props.loader, placeId, projectId);
  const [linkResult, setLinkResult] = useState<{ key: string; result: OpenResult } | null>(null);
  const { navigation, onEvent } = props;

  /** Navigate to a record's home view. Never a write, never an approval. */
  const openLink = useCallback((link: WorldRecordLink) => {
    const result = openRecordLink(link, navigation);
    const key = recordLinkKey(link);
    setLinkResult({ key, result });
    onEvent?.({ type: 'record_opened', placeId, recordKey: key, result });
    return result;
  }, [navigation, onEvent, placeId]);

  const stateLine = (sections: ReadonlyArray<Section<unknown> | undefined>) => {
    if (snap.failure) return { text: `Evidence could not be loaded: ${snap.failure}`, tone: 'error' as const };
    if (!snap.snapshot) return { text: 'Reading evidence…', tone: 'loading' as const };
    return snapshotStateLine(sections, now);
  };

  return { colors, styles, compact, now, snap, openLink, linkResult, stateLine };
}
