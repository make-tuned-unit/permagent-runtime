/**
 * Trace event recording — the pure logic behind the Execution trace buffer
 * (`useCommandCenter.events`, rendered by ExecutionTrace).
 *
 * Two wires feed the buffer (C3 nav-honesty fix — before this module the only
 * writer was the chat SSE 'Message' branch, so the trace showed nothing but the
 * user's own chat frames while its catalog entry promised the whole system):
 *
 *  • the GLOBAL `/events` WebSocket (opened by useAppNavigate): every frame the
 *    daemon broadcasts — navigations, agent/worker state, goal/decision/skill
 *    lifecycle, librarian runs, activity usage signals — is recorded with its
 *    real wire type. The daemon replays its buffer (≤1000 events) on every
 *    (re)connect, so global records are deduped by envelope id: history lands
 *    once, reconnect bursts are dropped.
 *
 *  • the PER-SESSION chat SSE (store.connectSession): Message/Error/Finish
 *    frames. A Message frame carrying toolRequest blocks is typed `tool_call`
 *    (that IS how a tool call fires on this wire); plain streaming deltas
 *    coalesce into the newest row so one long reply cannot evict the global
 *    rows from the bounded buffer.
 *
 * Store-free by design (`import type` only) so store.ts → traceEvents never
 * forms a runtime import cycle; the setState writers live at the two call
 * sites (useAppNavigate.recordGlobalTraceFrame, store.connectSession).
 */

import { wireEventType } from './wireEvent';
import type { EventRecord } from './store';
import type { SSEEvent, DaemonMessage } from './api';

/** Buffer cap — matches the store's MAX_EVENTS_BUFFER and the daemon's own
 *  replay buffer (EVENT_BUFFER_SIZE = 1000 in events/mod.rs). */
export const TRACE_EVENT_CAP = 1000;

/**
 * Frame types never recorded:
 *  • librarian_describe_token — a per-token firehose during a describe run; the
 *    run itself is recorded via librarian_describe_started/completed, and the
 *    token frames would evict everything else from the bounded buffer.
 *  • resume_gap — the /events socket's bookkeeping notice, not a runtime event.
 */
const SKIP_GLOBAL_TYPES = new Set(['librarian_describe_token', 'resume_gap']);

/** Wire types that are real failures — rendered with error severity. */
const ERROR_EVENT_TYPES = new Set([
  'task_failed',
  'integration_error',
  'activity:automation_job_failed',
]);

function optStr(v: unknown): string | null {
  return typeof v === 'string' && v.length > 0 ? v : null;
}

function asRecord(v: unknown): Record<string, unknown> {
  return v && typeof v === 'object' ? (v as Record<string, unknown>) : {};
}

/**
 * Global `/events` frame → trace record, or null for frames that must not be
 * recorded (unknown/malformed shapes, skip-listed types). The record keeps the
 * envelope's own id (dedupe key for reconnect replays) and timestamp, and
 * flattens the activity envelope (`type:'activity'`, inner event_type in
 * `payload.event.event_type`) to `activity:<inner>` so a usage signal reads as
 * the event that actually fired, with its provenance still visible.
 */
export function globalFrameToRecord(frame: unknown): EventRecord | null {
  const wireType = wireEventType(frame);
  if (!wireType || SKIP_GLOBAL_TYPES.has(wireType)) return null;
  const f = asRecord(frame);
  const id = optStr(f.id);
  if (!id) return null; // every PermagentEvent carries a UUIDv7; no id = not a bus event
  const payload = asRecord(f.payload);

  let eventType = wireType;
  if (wireType === 'activity') {
    const inner = optStr(asRecord(payload.event).event_type);
    if (inner) eventType = `activity:${inner}`;
  }

  return {
    id,
    timestamp: optStr(f.timestamp) ?? new Date().toISOString(),
    source: 'events',
    event_type: eventType,
    severity: ERROR_EVENT_TYPES.has(eventType) ? 'error' : 'info',
    run_id: null,
    task_id: optStr(payload.task_id),
    agent_id: optStr(payload.agent_id),
    correlation_id: optStr(payload.session_id),
    payload,
  };
}

/** Monotonic suffix so same-millisecond session rows never collide on id (the
 *  old writer's `sse-${Date.now()}` did, breaking React keys). */
let sessionSeq = 0;

/** Tolerant toolRequest name read — mirrors store.ts readToolCallInner (which
 *  is store-private; duplicating the 4 lines beats a runtime import cycle):
 *  the daemon wraps the call as `{ status, value:{ name } }` or serves it flat. */
function toolNames(msg: DaemonMessage | undefined): string[] {
  const names: string[] = [];
  for (const c of msg?.content ?? []) {
    if (asRecord(c).type !== 'toolRequest') continue;
    const tc = asRecord(asRecord(c).toolCall);
    const inner = 'value' in tc && tc.value ? asRecord(tc.value) : tc;
    names.push(optStr(inner.name) ?? 'unknown');
  }
  return names;
}

/**
 * Per-session SSE frame → trace record, or null for frames that are transport
 * bookkeeping (Ping, ActiveRequests). Message frames carrying toolRequest
 * blocks are typed `tool_call` with the tool names surfaced; Error/Finish are
 * the turn's real lifecycle signals.
 */
export function sessionFrameToRecord(frame: SSEEvent): EventRecord | null {
  if (frame.type !== 'Message' && frame.type !== 'Error' && frame.type !== 'Finish') return null;

  const base = {
    id: `sse-${Date.now()}-${sessionSeq++}`,
    timestamp: new Date().toISOString(),
    source: 'session',
    severity: frame.type === 'Error' ? 'error' : 'info',
    run_id: null,
    task_id: null,
    agent_id: null,
    correlation_id: null,
    payload: frame as unknown as Record<string, unknown>,
  };

  if (frame.type === 'Message') {
    const msg = (frame as { type: 'Message'; message?: DaemonMessage }).message;
    const tools = toolNames(msg);
    return {
      ...base,
      event_type: tools.length > 0 ? 'tool_call' : 'Message',
      correlation_id: optStr(msg?.id),
      payload: tools.length > 0 ? { ...base.payload, tools } : base.payload,
    };
  }
  return { ...base, event_type: frame.type };
}

/**
 * Prepend a record to the bounded buffer. Consecutive per-session 'Message'
 * rows (streaming text deltas of one reply) coalesce into the newest row —
 * the head keeps its identity (stable React key) and takes the fresh
 * timestamp/payload — so a long reply is one live row, not hundreds of rows
 * flushing the global events out of the cap.
 */
export function appendTraceRecord(
  events: EventRecord[],
  record: EventRecord,
  cap: number = TRACE_EVENT_CAP,
): EventRecord[] {
  const head = events[0];
  if (
    record.source === 'session' &&
    record.event_type === 'Message' &&
    head?.source === 'session' &&
    head.event_type === 'Message'
  ) {
    return [{ ...record, id: head.id }, ...events.slice(1)];
  }
  return [record, ...events].slice(0, cap);
}

// ── Global-frame dedupe ─────────────────────────────────────────────────────
// The daemon replays its whole buffer on every (re)connect; ids are claimed
// once so the first connect records history and reconnect bursts are dropped.
// Capacity comfortably exceeds the daemon's 1000-event replay buffer.
const DEDUPE_CAP = 2048;
const seenIds = new Set<string>();
const seenOrder: string[] = [];

/** Claim a global envelope id — true the first time, false on a replay. */
export function claimTraceEventId(id: string): boolean {
  if (seenIds.has(id)) return false;
  seenIds.add(id);
  seenOrder.push(id);
  while (seenOrder.length > DEDUPE_CAP) {
    const oldest = seenOrder.shift();
    if (oldest !== undefined) seenIds.delete(oldest);
  }
  return true;
}

/** Test-only: forget all claimed ids. */
export function _resetTraceDedupe(): void {
  seenIds.clear();
  seenOrder.length = 0;
}

/**
 * One-line human summary for a trace row — derived strictly from the payload
 * the event really carried (empty string when there is nothing useful to say).
 */
export function summarizeTraceEvent(rec: EventRecord): string {
  const p = rec.payload ?? {};
  let s = '';
  switch (rec.event_type) {
    case 'tool_call': {
      const tools = Array.isArray(p.tools) ? p.tools.filter((t): t is string => typeof t === 'string') : [];
      s = tools.join(', ');
      break;
    }
    case 'Error':
      s = optStr(p.error) ?? '';
      break;
    case 'Finish':
      s = optStr(p.reason) ?? '';
      break;
    case 'app_navigate':
      s = [optStr(p.tool_type) ?? optStr(p.tab), optStr(p.reason)].filter(Boolean).join(' — ');
      break;
    case 'agent_state_changed':
      s = [optStr(p.name) ?? optStr(p.agent_id), optStr(p.state)].filter(Boolean).join(': ');
      break;
    case 'goal_state_changed':
      s = [optStr(p.from) ?? 'new', optStr(p.to)].filter(Boolean).join(' → ');
      break;
    case 'memory_recalled': {
      const q = optStr(p.query);
      s = q ? `"${q}" (${typeof p.hit_count === 'number' ? p.hit_count : '?'} hits)` : '';
      break;
    }
    case 'task_failed':
      s = optStr(p.error) ?? '';
      break;
    case 'proactive_nudge':
      s = optStr(p.subject) ?? '';
      break;
    case 'librarian_describe_started':
    case 'librarian_describe_completed':
      s = optStr(p.memory_key) ?? '';
      break;
    default:
      if (rec.event_type.startsWith('activity:')) {
        s = optStr(asRecord(p.event).source_surface) ?? '';
      }
  }
  return s.length > 120 ? `${s.slice(0, 119)}…` : s;
}
