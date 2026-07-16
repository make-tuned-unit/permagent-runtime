// Decision petitions — the REAL Decision Inbox, physically waiting.
//
// Every pending decision is Henry formally asking the user something
// (/api/decisions, status='open'). The Petition Basin renders each one as a
// sealed scroll hovering over the water-stone bowl: a LIVE decision_created
// drops a new scroll in; answering one (decision_resolved) sends its scroll
// ascending out of the world. Zero pending ⇒ an empty, still basin — honest.
//
// Wire: slow reconciling poll (30s) + instant refetch on the real
// decision_created / decision_resolved events (shared worldEvents socket).
// The event is the trigger, the API stays the source of truth — the same
// pattern goalActivity uses for goal_state_changed.
//
// PURE CORE (unit-tested): diffPetitions — previous/next open id-lists in,
// {arrived, resolved} out — drives the drop-in / ascend animations without
// the renderer ever guessing.

import { useSyncExternalStore } from 'react';
import { apiFetch } from '../../../lib/api';
import { subscribeWorldEvents } from '../shared/worldEvents';

export interface Petition {
  id: string;
  /** The decision's real headline (plaque text on hover-scale renders). */
  headline: string;
  /** Epoch ms this petition was first seen by the world (drop-in anim clock). */
  seenAt: number;
  /** Set when resolved — the scroll ascends for ASCEND_MS then leaves the list. */
  resolvedAt?: number;
}

interface PetitionState {
  petitions: Petition[];
  /** Real total pending (may exceed the rendered cap — surfaced as "+N"). */
  totalPending: number;
  loaded: boolean;
}

/** The basin renders at most this many scrolls; beyond it an honest "+N". */
export const PETITION_CAP = 8;
export const ASCEND_MS = 1_600;

let state: PetitionState = { petitions: [], totalPending: 0, loaded: false };
const listeners = new Set<() => void>();

function publish(next: PetitionState): void {
  state = next;
  listeners.forEach((l) => l());
}

/** Pure diff (unit-tested): which ids arrived, which resolved. */
export function diffPetitions(
  prev: string[],
  next: string[],
): { arrived: string[]; resolved: string[] } {
  const prevSet = new Set(prev);
  const nextSet = new Set(next);
  return {
    arrived: next.filter((id) => !prevSet.has(id)),
    resolved: prev.filter((id) => !nextSet.has(id)),
  };
}

interface InboxItem {
  id?: string;
  headline?: string;
}

async function refetch(now: number = Date.now()): Promise<void> {
  try {
    const data = await apiFetch<{
      items?: InboxItem[];
      summary?: { total_pending?: number; totalPending?: number };
    }>('/api/decisions');
    const items = (data.items ?? []).filter((i): i is Required<InboxItem> => Boolean(i.id));
    const openIds = items.map((i) => i.id);
    const kept = state.petitions.filter((p) => !p.resolvedAt);
    const { arrived, resolved } = diffPetitions(
      kept.map((p) => p.id),
      openIds,
    );
    const headlineOf = new Map(items.map((i) => [i.id, i.headline ?? '']));
    const next: Petition[] = [];
    for (const p of state.petitions) {
      if (p.resolvedAt) {
        // Mid-ascent scrolls stay until their animation window closes.
        if (now - p.resolvedAt < ASCEND_MS) next.push(p);
      } else if (resolved.includes(p.id)) {
        next.push({ ...p, resolvedAt: now });
      } else {
        next.push(p);
      }
    }
    for (const id of arrived.slice(0, PETITION_CAP)) {
      next.push({ id, headline: headlineOf.get(id) ?? '', seenAt: now });
    }
    const totalPending =
      data.summary?.total_pending ?? data.summary?.totalPending ?? openIds.length;
    publish({ petitions: next.slice(-PETITION_CAP * 2), totalPending, loaded: true });
  } catch {
    // Daemon unreachable: keep the last true state — never flash or fabricate.
  }
}

/** Drop scrolls whose ascend animation has finished. Publishes only on change. */
export function sweepPetitions(now: number): void {
  const next = state.petitions.filter((p) => !p.resolvedAt || now - p.resolvedAt < ASCEND_MS);
  if (next.length !== state.petitions.length) {
    publish({ ...state, petitions: next });
  }
}

const POLL_MS = 30_000;
let started = false;

export function ensureDecisions(): void {
  if (started || typeof window === 'undefined') return;
  started = true;
  void refetch();
  setInterval(() => void refetch(), POLL_MS);
  // Live wire: a created/resolved decision refreshes within one round-trip.
  // Refetch is idempotent, so replayed buffer frames are harmless triggers.
  subscribeWorldEvents((evt) => {
    if (evt.type === 'decision_created' || evt.type === 'decision_resolved') {
      void refetch();
    }
  });
}

export function getPetitions(): PetitionState {
  return state;
}

export function usePetitions(): PetitionState {
  ensureDecisions();
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => state,
  );
}

// DEV-ONLY evidence harness (mirrors __worldSignals): stage honest-shaped
// petition states for capture without a live daemon. No-op in prod.
declare global {
  interface Window {
    __worldDecisions?: {
      set: (items: { id: string; headline: string }[]) => void;
      resolve: (id: string) => void;
    };
  }
}
if (import.meta.env.DEV && typeof window !== 'undefined') {
  window.__worldDecisions = {
    set: (items) => {
      const now = Date.now();
      publish({
        petitions: items
          .slice(0, PETITION_CAP)
          .map((i) => ({ id: i.id, headline: i.headline, seenAt: now })),
        totalPending: items.length,
        loaded: true,
      });
    },
    resolve: (id) => {
      const now = Date.now();
      publish({
        ...state,
        totalPending: Math.max(0, state.totalPending - 1),
        petitions: state.petitions.map((p) => (p.id === id ? { ...p, resolvedAt: now } : p)),
      });
    },
  };
}
