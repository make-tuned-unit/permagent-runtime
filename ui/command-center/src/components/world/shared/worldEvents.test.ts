// The shared /events wire is the single socket every world consumer reads. Its
// decode + replay classification is the honesty gate: buffered history (events
// stamped before the wire started) must be flagged `replayed` so state-claiming
// consumers skip it and the world only ever animates work it witnessed live.

import { describe, expect, it } from 'vitest';
import { decodeWireFrame, isReplayed } from './worldEvents';

describe('isReplayed', () => {
  const startedAt = 1_000_000;
  it('flags events stamped before the wire started', () => {
    expect(isReplayed(new Date(startedAt - 5000).toISOString(), startedAt)).toBe(true);
  });
  it('treats live (post-start) events as fresh', () => {
    expect(isReplayed(new Date(startedAt + 5000).toISOString(), startedAt)).toBe(false);
  });
  it('treats an unparseable/absent timestamp as live (daemon always stamps)', () => {
    expect(isReplayed(undefined, startedAt)).toBe(false);
    expect(isReplayed('not-a-date', startedAt)).toBe(false);
  });
});

describe('decodeWireFrame', () => {
  const startedAt = 1_000_000;

  it('decodes the envelope shape (type via rename, payload passthrough)', () => {
    const raw = JSON.stringify({
      id: 'e1',
      type: 'task_completed',
      timestamp: new Date(startedAt + 1000).toISOString(),
      payload: { task_id: 't-9' },
    });
    const evt = decodeWireFrame(raw, startedAt)!;
    expect(evt.type).toBe('task_completed');
    expect(evt.payload).toEqual({ task_id: 't-9' });
    expect(evt.id).toBe('e1');
    expect(evt.replayed).toBe(false);
  });

  it('resolves the inner event_type field when there is no top-level type', () => {
    const raw = JSON.stringify({ event_type: 'librarian_describe_started', timestamp: null });
    expect(decodeWireFrame(raw, startedAt)!.type).toBe('librarian_describe_started');
  });

  it('marks buffered history as replayed', () => {
    const raw = JSON.stringify({
      type: 'memory_added',
      timestamp: new Date(startedAt - 60_000).toISOString(),
    });
    expect(decodeWireFrame(raw, startedAt)!.replayed).toBe(true);
  });

  it('returns null on malformed frames and typeless payloads', () => {
    expect(decodeWireFrame('{not json', startedAt)).toBeNull();
    expect(decodeWireFrame('42', startedAt)).toBeNull();
    expect(decodeWireFrame(JSON.stringify({ foo: 'bar' }), startedAt)).toBeNull(); // no type
  });

  it('defaults a missing/non-object payload to an empty object', () => {
    const raw = JSON.stringify({ type: 'proactive_nudge' });
    expect(decodeWireFrame(raw, startedAt)!.payload).toEqual({});
  });
});
