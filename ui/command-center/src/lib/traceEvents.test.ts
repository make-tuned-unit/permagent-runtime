/**
 * Trace event recording (C3 nav-honesty) — pure-logic tests.
 *
 * The Execution trace's catalog entry promises tool calls, worker activity,
 * navigations and lifecycle signals off the global /events bus; these tests
 * pin the recording layer that makes that true: global frames become records
 * with their REAL wire types (activity envelopes flattened, firehose token
 * frames skipped, replay bursts deduped by envelope id), and per-session SSE
 * frames are typed tool_call/Message/Error/Finish with streaming text deltas
 * coalescing so one long reply cannot evict the global rows from the cap.
 */

import { describe, expect, it, beforeEach } from 'vitest';
import {
  appendTraceRecord,
  claimTraceEventId,
  globalFrameToRecord,
  sessionFrameToRecord,
  summarizeTraceEvent,
  _resetTraceDedupe,
  TRACE_EVENT_CAP,
} from './traceEvents';
import type { EventRecord } from './store';
import type { SSEEvent, DaemonMessage } from './api';

function busFrame(type: string, payload: Record<string, unknown> = {}, id = `id-${type}`): unknown {
  return { id, type, timestamp: '2026-07-18T10:00:00Z', payload };
}

function msg(content: DaemonMessage['content']): DaemonMessage {
  return { id: 'msg-1', role: 'assistant', created: 0, content, metadata: { userVisible: true, agentVisible: true } };
}

function messageFrame(content: DaemonMessage['content']): SSEEvent {
  return { type: 'Message', message: msg(content) } as unknown as SSEEvent;
}

beforeEach(() => _resetTraceDedupe());

describe('globalFrameToRecord — real wire types off the /events bus', () => {
  it('records a navigation frame with its real type, id and timestamp', () => {
    const rec = globalFrameToRecord(busFrame('app_navigate', { tool_type: 'build', reason: 'r' }, 'uuid-1'));
    expect(rec).toMatchObject({
      id: 'uuid-1',
      event_type: 'app_navigate',
      source: 'events',
      severity: 'info',
      timestamp: '2026-07-18T10:00:00Z',
    });
  });

  it('records worker/lifecycle frames typed as themselves', () => {
    for (const t of ['agent_state_changed', 'goal_state_changed', 'decision_created', 'skill_triggered', 'daemon_started', 'librarian_describe_started', 'memory_recalled']) {
      expect(globalFrameToRecord(busFrame(t))?.event_type).toBe(t);
    }
  });

  it('flattens the activity envelope to activity:<inner event>', () => {
    const rec = globalFrameToRecord(
      busFrame('activity', { channel: 'activity', event: { event_type: 'terminal_command_completed', source_surface: 'terminal' } }),
    );
    expect(rec?.event_type).toBe('activity:terminal_command_completed');
  });

  it('keeps a malformed activity envelope as plain activity', () => {
    expect(globalFrameToRecord(busFrame('activity', { channel: 'activity' }))?.event_type).toBe('activity');
  });

  it('skips the librarian per-token firehose but keeps the run lifecycle', () => {
    expect(globalFrameToRecord(busFrame('librarian_describe_token'))).toBeNull();
    expect(globalFrameToRecord(busFrame('librarian_describe_completed'))).not.toBeNull();
  });

  it('skips resume_gap bookkeeping and malformed frames', () => {
    expect(globalFrameToRecord({ type: 'resume_gap', message: 'gap' })).toBeNull();
    expect(globalFrameToRecord({ payload: {} })).toBeNull(); // no type
    expect(globalFrameToRecord({ type: 'app_navigate' })).toBeNull(); // no envelope id
    expect(globalFrameToRecord('not-an-object')).toBeNull();
  });

  it('marks real failures with error severity', () => {
    expect(globalFrameToRecord(busFrame('task_failed', { error: 'boom' }))?.severity).toBe('error');
    expect(
      globalFrameToRecord(busFrame('activity', { event: { event_type: 'automation_job_failed' } }))?.severity,
    ).toBe('error');
  });

  it('lifts task/agent/session ids out of the payload', () => {
    const rec = globalFrameToRecord(busFrame('task_started', { task_id: 't1', session_id: 's1' }));
    expect(rec?.task_id).toBe('t1');
    expect(rec?.correlation_id).toBe('s1');
  });
});

describe('sessionFrameToRecord — per-session SSE typing', () => {
  it('types a plain streaming Message frame as Message with its message id', () => {
    const rec = sessionFrameToRecord(messageFrame([{ type: 'text', text: 'hi' } as never]));
    expect(rec?.event_type).toBe('Message');
    expect(rec?.source).toBe('session');
    expect(rec?.correlation_id).toBe('msg-1');
  });

  it('types a toolRequest-bearing Message frame as tool_call with the tool names (wrapped + flat shapes)', () => {
    const wrapped = { type: 'toolRequest', id: 'r1', toolCall: { status: 'success', value: { name: 'shell', arguments: {} } } };
    const flat = { type: 'toolRequest', id: 'r2', toolCall: { name: 'web_search', arguments: {} } };
    const rec = sessionFrameToRecord(messageFrame([wrapped, flat] as never));
    expect(rec?.event_type).toBe('tool_call');
    expect(rec?.payload.tools).toEqual(['shell', 'web_search']);
  });

  it('records Error frames with error severity and Finish frames as lifecycle', () => {
    const err = sessionFrameToRecord({ type: 'Error', error: 'provider down' } as SSEEvent);
    expect(err).toMatchObject({ event_type: 'Error', severity: 'error' });
    const fin = sessionFrameToRecord({ type: 'Finish', reason: 'stop' } as unknown as SSEEvent);
    expect(fin).toMatchObject({ event_type: 'Finish', severity: 'info' });
  });

  it('ignores transport bookkeeping frames (Ping, ActiveRequests)', () => {
    expect(sessionFrameToRecord({ type: 'Ping' } as SSEEvent)).toBeNull();
    expect(sessionFrameToRecord({ type: 'ActiveRequests', request_ids: [] } as unknown as SSEEvent)).toBeNull();
  });

  it('never collides ids for same-millisecond frames (the old sse-${Date.now()} bug)', () => {
    const a = sessionFrameToRecord({ type: 'Finish', reason: 'a' } as unknown as SSEEvent);
    const b = sessionFrameToRecord({ type: 'Finish', reason: 'b' } as unknown as SSEEvent);
    expect(a?.id).not.toBe(b?.id);
  });
});

describe('appendTraceRecord — bounded buffer + delta coalescing', () => {
  const mk = (over: Partial<EventRecord>): EventRecord => ({
    id: 'x', timestamp: 't', source: 'events', event_type: 'e', severity: 'info',
    run_id: null, task_id: null, agent_id: null, correlation_id: null, payload: {},
    ...over,
  });

  it('prepends newest-first and enforces the cap', () => {
    let events: EventRecord[] = [];
    for (let i = 0; i < TRACE_EVENT_CAP + 5; i++) {
      events = appendTraceRecord(events, mk({ id: `g-${i}` }));
    }
    expect(events).toHaveLength(TRACE_EVENT_CAP);
    expect(events[0].id).toBe(`g-${TRACE_EVENT_CAP + 4}`);
  });

  it('coalesces consecutive session Message rows into one live row (stable id, fresh timestamp)', () => {
    let events: EventRecord[] = [];
    events = appendTraceRecord(events, mk({ id: 's-1', source: 'session', event_type: 'Message', timestamp: 't1' }));
    events = appendTraceRecord(events, mk({ id: 's-2', source: 'session', event_type: 'Message', timestamp: 't2' }));
    expect(events).toHaveLength(1);
    expect(events[0].id).toBe('s-1'); // stable React key
    expect(events[0].timestamp).toBe('t2'); // latest activity
  });

  it('does NOT coalesce across types or sources — global rows survive a streaming reply', () => {
    let events: EventRecord[] = [];
    events = appendTraceRecord(events, mk({ id: 's-1', source: 'session', event_type: 'Message' }));
    events = appendTraceRecord(events, mk({ id: 'g-1', source: 'events', event_type: 'app_navigate' }));
    events = appendTraceRecord(events, mk({ id: 's-2', source: 'session', event_type: 'Message' }));
    events = appendTraceRecord(events, mk({ id: 's-3', source: 'session', event_type: 'tool_call' }));
    expect(events.map(e => e.id)).toEqual(['s-3', 's-2', 'g-1', 's-1']);
  });
});

describe('claimTraceEventId — replay-burst dedupe', () => {
  it('claims an id once and rejects the replay', () => {
    expect(claimTraceEventId('a')).toBe(true);
    expect(claimTraceEventId('a')).toBe(false);
    expect(claimTraceEventId('b')).toBe(true);
  });

  it('evicts oldest ids beyond capacity so memory stays bounded', () => {
    for (let i = 0; i < 2049; i++) claimTraceEventId(`id-${i}`);
    // id-0 has been evicted from the dedupe window and is claimable again.
    expect(claimTraceEventId('id-0')).toBe(true);
    // A recent id is still remembered.
    expect(claimTraceEventId('id-2048')).toBe(false);
  });
});

describe('summarizeTraceEvent — payload-derived row summaries', () => {
  const mk = (event_type: string, payload: Record<string, unknown>): EventRecord => ({
    id: 'x', timestamp: 't', source: 'events', event_type, severity: 'info',
    run_id: null, task_id: null, agent_id: null, correlation_id: null, payload,
  });

  it('summarizes the high-value types from what the event really carried', () => {
    expect(summarizeTraceEvent(mk('tool_call', { tools: ['shell', 'edit'] }))).toBe('shell, edit');
    expect(summarizeTraceEvent(mk('Error', { error: 'provider down' }))).toBe('provider down');
    expect(summarizeTraceEvent(mk('app_navigate', { tool_type: 'build', reason: 'opening' }))).toBe('build — opening');
    expect(summarizeTraceEvent(mk('agent_state_changed', { name: 'Henry', state: 'working' }))).toBe('Henry: working');
    expect(summarizeTraceEvent(mk('goal_state_changed', { from: 'queued', to: 'running' }))).toBe('queued → running');
    expect(summarizeTraceEvent(mk('activity:terminal_command_completed', { event: { source_surface: 'terminal' } }))).toBe('terminal');
  });

  it('returns empty for types with nothing useful and truncates long summaries', () => {
    expect(summarizeTraceEvent(mk('daemon_started', { version: '1' }))).toBe('');
    const long = summarizeTraceEvent(mk('Error', { error: 'x'.repeat(300) }));
    expect(long.length).toBe(120);
    expect(long.endsWith('…')).toBe(true);
  });
});
