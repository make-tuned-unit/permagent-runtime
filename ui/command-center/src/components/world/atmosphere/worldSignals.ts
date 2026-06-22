// Atmosphere — REAL Brain/event signals for the atmosphere detail pass.
//
// Honesty law: water IS the user; every glow is a real daemon event; no faked
// activity, no simulated peers. This module is the honesty boundary for the
// atmosphere lane's content bindings — the spring, recall-as-river and
// rainfall-on-ingestion all read from HERE and nowhere else, so the rule
// "render the geometry dormant when there is no real signal" reaches the pixels
// regardless of caller. (The day-one silhouette entities below currently have no
// consumer — their renderer, ShadowsOnTheWall, was removed in #418; the data is
// left in place as a follow-up cleanup, not load-bearing.)
//
// Inputs (all READ-ONLY, all already consumed by the command-center frontend —
// this lane adds NO daemon code and NO new endpoints):
//   • /api/brain/graph        → entities[].length, memories[].length
//     (the real near-empty-Brain truth for the spring presence + day-one shadows)
//   • /events  memory_added   → rainfall-on-ingestion pulse
//   • /events  librarian_describe_* / task_* → recall-as-river flow pulses
//
// What does NOT exist today (gaps filed in the PR, NOT faked here):
//   • no recall_* event — recall-as-river is approximated from describe/task
//     activity; a dedicated `recall` event is the proposed daemon follow-up.
//   • no entity_added event — the silhouette set refreshes on the graph poll.
//
// LAW: zero per-frame allocations downstream. This module exposes a
// plain-getter snapshot (getWorldSignals) for useFrame loops and a
// useSyncExternalStore hook for React consumers. The store is updated only on
// real network events / polls, never per frame.

import { useSyncExternalStore } from 'react';
import { apiFetch, getApiBaseUrl } from '../../../lib/api';
import { wireEventType } from '../../../lib/wireEvent';

/** A real Brain entity (formerly projected as a day-one shadow silhouette). */
export interface SignalEntity {
  id: string;
  /** Spectral entity type (person, project, tool, …) — drives silhouette shape. */
  type: string;
  name: string;
}

export interface WorldSignals {
  /** Real entity count from /api/brain/graph. 0 ⇒ render dormant. */
  entityCount: number;
  /** Real memory count. The spring rises on the Brain's FIRST memory (> 0). */
  memoryCount: number;
  /** Real entities for the day-one shadow silhouettes (capped for the budget). */
  entities: SignalEntity[];
  /**
   * Event-driven flow level in [0,1], decayed in JS-time (NOT per-frame): bumped
   * by memory_added (rainfall) and describe/task activity (recall-as-river).
   * Consumers read this each frame to brighten the river / trigger ripples.
   */
  flow: number;
  /** Monotonic counter — bumped on each memory_added. Lets the river fire a ripple. */
  rainTick: number;
  /** True once a successful /api/brain/graph read has happened (vs. daemon down). */
  loaded: boolean;
}

const SILHOUETTE_CAP = 12; // budget cap on the (currently unconsumed) silhouette entity set

let signals: WorldSignals = {
  entityCount: 0,
  memoryCount: 0,
  entities: [],
  flow: 0,
  rainTick: 0,
  loaded: false,
};

const listeners = new Set<() => void>();

function publish(next: Partial<WorldSignals>): void {
  signals = { ...signals, ...next };
  listeners.forEach((l) => l());
}

/** Non-hook accessor for useFrame loops (no per-frame React work, no allocation). */
export function getWorldSignals(): WorldSignals {
  return signals;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useWorldSignals(): WorldSignals {
  return useSyncExternalStore(subscribe, () => signals);
}

// --- Flow decay (JS-time, event-driven; consumers read the level per frame) ---
// We decay on a coarse interval rather than per frame so this module owns no
// useFrame loop. A memory_added bumps flow to 1; describe/task activity bumps it
// to ~0.6; it eases back to 0 over a few seconds — recall-as-river running, then
// the channel settling. The agents never dam water; this only modulates
// brightness/ripple, never gates presence (presence = memoryCount).
const FLOW_DECAY_PER_TICK = 0.04;
const FLOW_TICK_MS = 120;

function bumpFlow(to: number): void {
  if (to > signals.flow) publish({ flow: Math.min(1, to) });
}

// --- Brain graph poll (presence: spring) ---
interface BrainGraphResponse {
  entities?: { id: string; type: string; name: string }[];
  memories?: unknown[];
}

const GRAPH_POLL_MS = 60_000; // matches useBrainData's cadence — slow is sacred

let started = false;
let pollTimer: ReturnType<typeof setInterval> | undefined;
let decayTimer: ReturnType<typeof setInterval> | undefined;
let ws: WebSocket | null = null;
let wsRetry: ReturnType<typeof setTimeout> | undefined;
let entityRefreshTimer: ReturnType<typeof setTimeout> | undefined;
let closed = false;

// Event-driven graph refresh: an entity_added/_updated means the graph changed.
// Coalesce bursts (one annotation can surface several entities) into a single
// poll ~400ms later, so updates appear within ~½s instead of waiting up to 60s.
// The slow GRAPH_POLL_MS timer stays as the reconciliation backstop.
function scheduleGraphRefresh(): void {
  if (entityRefreshTimer) return;
  entityRefreshTimer = setTimeout(() => {
    entityRefreshTimer = undefined;
    void pollGraph();
  }, 400);
}

async function pollGraph(): Promise<void> {
  try {
    const g = await apiFetch<BrainGraphResponse>('/api/brain/graph');
    const entities = (g.entities ?? []).slice(0, SILHOUETTE_CAP).map((e) => ({
      id: e.id,
      type: e.type,
      name: e.name,
    }));
    publish({
      entityCount: g.entities?.length ?? 0,
      memoryCount: g.memories?.length ?? 0,
      entities,
      loaded: true,
    });
  } catch {
    // Daemon unreachable: keep the last known truth (or dormant) — do NOT fake.
    publish({ loaded: true });
  }
}

function connectEvents(): void {
  if (closed) return;
  const base = getApiBaseUrl().replace(/^http/, 'ws');
  const mountedAt = Date.now();
  try {
    ws = new WebSocket(`${base}/events`);
  } catch {
    return;
  }
  ws.onmessage = (ev) => {
    let parsed: { type?: string; event_type?: string; timestamp?: string };
    try {
      parsed = JSON.parse(ev.data);
    } catch {
      return;
    }
    const type = wireEventType(parsed);
    const ts = Date.parse(parsed.timestamp ?? '');
    if (Number.isFinite(ts) && ts < mountedAt) return; // skip replayed buffer
    if (type === 'memory_added') {
      // Rainfall-on-ingestion: a real new memory. Bump flow + the rain tick, and
      // optimistically reflect the new memory so the spring can rise immediately
      // (the next graph poll reconciles the exact count).
      publish({
        rainTick: signals.rainTick + 1,
        memoryCount: signals.memoryCount + 1,
      });
      bumpFlow(1);
    } else if (type === 'memory_recalled') {
      // Recall-as-river: a real Brain recall happened. This is the genuine
      // recall signal (#324) — the river flows on actual retrieval, not a proxy.
      bumpFlow(1);
    } else if (type === 'entity_added' || type === 'entity_updated') {
      // A real entity surfaced on the graph (#325). Refresh now (event-driven)
      // instead of waiting up to 60s; the API stays the source of truth for
      // entity shape/dedup/caps.
      scheduleGraphRefresh();
    } else if (
      type.startsWith('librarian_describe') ||
      type.startsWith('task_')
    ) {
      // Ambient activity: describe/task work keeps the channel faintly running
      // between real recalls.
      bumpFlow(0.6);
    }
  };
  ws.onerror = () => ws?.close();
  ws.onclose = () => {
    if (!closed) wsRetry = setTimeout(connectEvents, 3000);
  };
}

/**
 * Start the read-only signal subscriptions. Idempotent; safe to call from a
 * component mount. No-op on the server. Returns a disposer.
 */
export function startWorldSignals(): () => void {
  if (started || typeof window === 'undefined') return () => {};
  started = true;
  closed = false;

  void pollGraph();
  pollTimer = setInterval(() => void pollGraph(), GRAPH_POLL_MS);

  decayTimer = setInterval(() => {
    if (signals.flow > 0) {
      publish({ flow: Math.max(0, signals.flow - FLOW_DECAY_PER_TICK) });
    }
  }, FLOW_TICK_MS);

  connectEvents();

  return () => {
    closed = true;
    started = false;
    if (pollTimer) clearInterval(pollTimer);
    if (decayTimer) clearInterval(decayTimer);
    if (wsRetry) clearTimeout(wsRetry);
    if (entityRefreshTimer) clearTimeout(entityRefreshTimer);
    ws?.close();
    ws = null;
  };
}

// --- DEV-ONLY evidence harness ---------------------------------------------
// Lets CDP capture demonstrate the honest states without a live Brain:
//   • dormant (entityCount/memoryCount 0) vs flowing (real-shaped counts)
//   • the day-one near-empty wall vs a populated wall
// This sets the SAME store the real bindings write — it does not bypass the
// honesty boundary, it stands in for the daemon during capture. No-op in prod.
declare global {
  interface Window {
    __worldSignals?: {
      setBrain: (entityCount: number, memoryCount: number) => void;
      setEntities: (entities: SignalEntity[]) => void;
      rain: () => void;
      reset: () => void;
      /** Evidence: read the live store (shows the real binding shape in capture). */
      snapshot: () => WorldSignals;
    };
  }
}
if (import.meta.env.DEV && typeof window !== 'undefined') {
  window.__worldSignals = {
    setBrain: (entityCount, memoryCount) =>
      publish({ entityCount, memoryCount, loaded: true }),
    setEntities: (entities) =>
      publish({
        entities: entities.slice(0, SILHOUETTE_CAP),
        entityCount: entities.length,
        loaded: true,
      }),
    rain: () => {
      publish({ rainTick: signals.rainTick + 1, memoryCount: signals.memoryCount + 1 });
      bumpFlow(1);
    },
    reset: () =>
      publish({ entityCount: 0, memoryCount: 0, entities: [], flow: 0, loaded: true }),
    snapshot: () => signals,
  };
}
