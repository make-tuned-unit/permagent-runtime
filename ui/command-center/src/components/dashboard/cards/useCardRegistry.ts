import { useState, useEffect, useMemo } from 'react';
import { apiFetch } from '../../../lib/api';
import { CARD_REGISTRY, mergeRegistry, type CardManifest, type CardRegistryEntry } from './registry';

/**
 * Fetch the daemon-served card manifests (`GET /api/dashboard/card-types`).
 *
 * These are the declaratively-registered card types (issue #182): the daemon's
 * built-ins today, skill-pack contributions later. A failure or empty list is
 * non-fatal — the dashboard still has every first-party code card.
 */
export function useCardManifests(): CardManifest[] {
  const [manifests, setManifests] = useState<CardManifest[]>([]);

  useEffect(() => {
    let cancelled = false;
    apiFetch<CardManifest[]>('/api/dashboard/card-types')
      .then(list => { if (!cancelled && Array.isArray(list)) setManifests(list); })
      .catch(() => { /* no manifest cards — first-party registry stands alone */ });
    return () => { cancelled = true; };
  }, []);

  return manifests;
}

/**
 * The rendered card registry the Dashboard and AddCardPicker consume: the
 * first-party code cards merged with the daemon-served manifest cards. Recomputes
 * only when the manifest list actually changes.
 */
export function useCardRegistry(): Record<string, CardRegistryEntry> {
  const manifests = useCardManifests();
  return useMemo(() => mergeRegistry(CARD_REGISTRY, manifests), [manifests]);
}
