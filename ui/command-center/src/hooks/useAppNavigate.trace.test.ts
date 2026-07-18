/**
 * Global /events → trace buffer bridge (C3 nav-honesty) — wiring tests.
 *
 * The /events WebSocket handler routes nav frames AND mirrors every frame into
 * `useCommandCenter.events` via recordGlobalTraceFrame; this drives that seam
 * exactly as ws.onmessage does (parsed frame in, store rows out) and asserts
 * the trace genuinely gains typed global rows: navigations, worker activity,
 * lifecycle — with the daemon's replay-on-reconnect burst deduped by envelope
 * id and firehose token frames kept out.
 *
 * `../lib/api` is mocked (the openItem-test pattern) so importing the store +
 * hook module touches no network.
 */

import { describe, expect, it, beforeEach, vi } from 'vitest';

vi.mock('../lib/api', () => ({
  api: {},
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { recordGlobalTraceFrame } from './useAppNavigate';
import { useCommandCenter } from '../lib/store';
import { _resetTraceDedupe } from '../lib/traceEvents';

beforeEach(() => {
  _resetTraceDedupe();
  useCommandCenter.setState({ events: [] });
});

function frame(type: string, payload: Record<string, unknown>, id: string): unknown {
  return { id, type, timestamp: '2026-07-18T10:00:00Z', payload };
}

describe('recordGlobalTraceFrame — the trace sees the running system', () => {
  it('records nav / worker / decision / goal / librarian frames with their real types', () => {
    recordGlobalTraceFrame(frame('app_navigate', { tool_type: 'build', reason: 'r' }, 'e1'));
    recordGlobalTraceFrame(frame('agent_state_changed', { agent_id: 'henry', name: 'Henry', state: 'working' }, 'e2'));
    recordGlobalTraceFrame(frame('decision_created', { decision_id: 'd', kind: 'k', tier: 1 }, 'e3'));
    recordGlobalTraceFrame(frame('goal_state_changed', { goal_id: 'g', from: 'queued', to: 'running' }, 'e4'));
    recordGlobalTraceFrame(frame('librarian_describe_completed', { memory_key: 'note:x' }, 'e5'));
    recordGlobalTraceFrame(frame('activity', { channel: 'activity', event: { event_type: 'chat_turn_completed', source_surface: 'chat' } }, 'e6'));

    const types = useCommandCenter.getState().events.map(e => e.event_type);
    expect(types).toEqual([
      'activity:chat_turn_completed',
      'librarian_describe_completed',
      'goal_state_changed',
      'decision_created',
      'agent_state_changed',
      'app_navigate',
    ]);
  });

  it('dedupes the daemon replay burst by envelope id (reconnect records nothing twice)', () => {
    const f = frame('memory_recalled', { query: 'q', hit_count: 2, source: 's' }, 'stable-id');
    recordGlobalTraceFrame(f);
    recordGlobalTraceFrame(f); // replayed on reconnect
    expect(useCommandCenter.getState().events).toHaveLength(1);
  });

  it('drops firehose token frames and malformed frames without touching the buffer', () => {
    recordGlobalTraceFrame(frame('librarian_describe_token', { token: 'x' }, 'tok-1'));
    recordGlobalTraceFrame({ type: 'resume_gap', message: 'gap' });
    recordGlobalTraceFrame({ not: 'an event' });
    expect(useCommandCenter.getState().events).toHaveLength(0);
  });

  it('keeps the frame payload on the record for inspection', () => {
    recordGlobalTraceFrame(frame('proactive_nudge', { kind: 'brain', subject: 'dormant thread' }, 'e9'));
    expect(useCommandCenter.getState().events[0].payload).toMatchObject({ subject: 'dormant thread' });
  });
});
