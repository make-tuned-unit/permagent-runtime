import { useState, useEffect, useMemo } from 'react';
import { apiFetch } from '../../../lib/api';
import { CARD_REGISTRY, mergeRegistry, type CardManifest, type CardRegistryEntry } from './registry';

export type ManifestStatus = 'loading' | 'ready' | 'error';

/**
 * Fetch the daemon-served card manifests (`GET /api/dashboard/card-types`).
 *
 * These are the declaratively-registered card types (issue #182): the daemon's
 * built-ins today, skill-pack contributions later. A failure or empty list is
 * non-fatal — the dashboard still has every first-party code card.
 *
 * The status is returned because the *consumer's* behaviour depends on it. Two
 * cards in the default layout (Calendar, Council) are manifest-served, so until
 * this lands the Dashboard is asked to render a card type it has no entry for,
 * and its answer was `return null` — the card silently disappearing, leaving a
 * hole in the grid and no way to tell "still loading" from "the daemon is down"
 * from "that card type no longer exists". Reset to default made it worse: it
 * put both cards back into the layout, where they rendered as nothing.
 */
export function useCardManifests(): { manifests: CardManifest[]; status: ManifestStatus } {
  const [manifests, setManifests] = useState<CardManifest[]>([]);
  const [status, setStatus] = useState<ManifestStatus>('loading');

  useEffect(() => {
    let cancelled = false;
    apiFetch<CardManifest[]>('/api/dashboard/card-types')
      .then(list => {
        if (cancelled) return;
        if (Array.isArray(list)) setManifests(list);
        setStatus('ready');
      })
      .catch(() => { if (!cancelled) setStatus('error'); });
    return () => { cancelled = true; };
  }, []);

  return { manifests, status };
}

/**
 * The rendered card registry the Dashboard and AddCardPicker consume: the
 * first-party code cards merged with the daemon-served manifest cards. Recomputes
 * only when the manifest list actually changes.
 */
export function useCardRegistry(): {
  registry: Record<string, CardRegistryEntry>;
  status: ManifestStatus;
} {
  const { manifests, status } = useCardManifests();
  const registry = useMemo(() => mergeRegistry(CARD_REGISTRY, manifests), [manifests]);
  return { registry, status };
}
