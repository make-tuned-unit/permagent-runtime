// Shared /events wire for the whole World View.
//
// Before this module the world opened THREE parallel WebSockets to the same
// daemon /events feed (worldSignals, stateSources, goalActivity) — and every
// new real-signal consumer would have added a fourth. This is the single
// socket: consumers subscribe with a listener and receive every decoded wire
// event plus a `replayed` honesty flag (the daemon replays its buffer on
// connect; most consumers must ignore history so the world only ever animates
// work it witnessed live — see stateSources' Librarian rule).
//
// Semantics preserved from the strictest prior consumer (stateSources):
//   • `startedAt` is captured ONCE when the wire first starts and is stable
//     across reconnects, so a mid-session reconnect's replay burst is still
//     marked replayed instead of re-animating old work.
//   • Reconnect with a flat 3s retry (same cadence as before).
//   • Malformed frames are dropped silently.
//
// LAW: this module does no per-frame work; it only fans out network events.

import { getApiBaseUrl } from '../../../lib/api';
import { wireEventType } from '../../../lib/wireEvent';

export interface WorldWireEvent {
  /** Canonical snake_case wire type (via wireEventType). */
  type: string;
  /** Event payload object ({} when absent). */
  payload: Record<string, unknown>;
  /** Envelope id, when present. */
  id?: string;
  /** Raw RFC3339 timestamp string, when present. */
  timestamp?: string;
  /**
   * True when this event predates the wire's first start — i.e. it arrived in
   * a replayed daemon buffer, not live. State-claiming consumers skip these.
   */
  replayed: boolean;
}

type Listener = (evt: WorldWireEvent) => void;

const listeners = new Set<Listener>();
let ws: WebSocket | null = null;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let running = false;
/** Stable across reconnects — see module doc. */
let startedAt = 0;

const RETRY_MS = 3000;

/**
 * Pure classification helper (unit-tested): an event is replayed when its
 * timestamp parses and predates the wire start. Untimestamped events are
 * treated as live (the daemon stamps everything; absence means a test frame).
 */
export function isReplayed(timestamp: string | undefined, wireStartedAt: number): boolean {
  const ts = Date.parse(timestamp ?? '');
  return Number.isFinite(ts) && ts < wireStartedAt;
}

/** Pure decode of one raw frame → WorldWireEvent (null on malformed input). */
export function decodeWireFrame(raw: string, wireStartedAt: number): WorldWireEvent | null {
  let parsed: {
    type?: string;
    event_type?: string;
    id?: string;
    timestamp?: string;
    payload?: unknown;
  };
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof parsed !== 'object' || parsed === null) return null;
  const type = wireEventType(parsed);
  if (!type) return null;
  const payload =
    typeof parsed.payload === 'object' && parsed.payload !== null
      ? (parsed.payload as Record<string, unknown>)
      : {};
  return {
    type,
    payload,
    id: parsed.id,
    timestamp: parsed.timestamp,
    replayed: isReplayed(parsed.timestamp, wireStartedAt),
  };
}

function connect(): void {
  if (!running) return;
  const base = getApiBaseUrl().replace(/^http/, 'ws');
  try {
    ws = new WebSocket(`${base}/events`);
  } catch {
    retryTimer = setTimeout(connect, RETRY_MS);
    return;
  }
  ws.onmessage = (ev) => {
    const evt = decodeWireFrame(String(ev.data), startedAt);
    if (!evt) return;
    listeners.forEach((l) => {
      try {
        l(evt);
      } catch {
        // One consumer's throw must not starve the others.
      }
    });
  };
  ws.onerror = () => ws?.close();
  ws.onclose = () => {
    ws = null;
    if (running) retryTimer = setTimeout(connect, RETRY_MS);
  };
}

function start(): void {
  if (running || typeof window === 'undefined') return;
  running = true;
  if (startedAt === 0) startedAt = Date.now();
  connect();
}

function stop(): void {
  running = false;
  if (retryTimer) clearTimeout(retryTimer);
  retryTimer = undefined;
  ws?.close();
  ws = null;
}

/**
 * Subscribe to the shared wire. The socket opens on the first subscriber and
 * closes when the last unsubscribes (long-lived stores that never unsubscribe
 * simply keep it open for the app's life — the goalActivity precedent).
 */
export function subscribeWorldEvents(listener: Listener): () => void {
  listeners.add(listener);
  if (listeners.size === 1) start();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) stop();
  };
}

// DEV-ONLY evidence harness: inject a synthetic live event into the fanout so
// CDP capture can demonstrate honest event-driven behavior without a live
// daemon. Sets the SAME path real frames take (post-decode). No-op in prod.
declare global {
  interface Window {
    __worldEvents?: {
      emit: (type: string, payload?: Record<string, unknown>) => void;
    };
  }
}
if (import.meta.env.DEV && typeof window !== 'undefined') {
  window.__worldEvents = {
    emit: (type, payload = {}) => {
      const evt: WorldWireEvent = {
        type,
        payload,
        timestamp: new Date().toISOString(),
        replayed: false,
      };
      listeners.forEach((l) => l(evt));
    },
  };
}
