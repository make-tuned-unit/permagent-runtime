// The shared /events wire is the single socket every world consumer reads. Its
// decode + replay classification is the honesty gate: buffered history (events
// stamped before the wire started) must be flagged `replayed` so state-claiming
// consumers skip it and the world only ever animates work it witnessed live.

import { describe, expect, it } from 'vitest';
import { decodeWireFrame, frameReplayed, isReplayed } from './worldEvents';

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

describe('frameReplayed — server marker preferred, epoch fallback', () => {
  const startedAt = 1_000_000;
  const liveTs = new Date(startedAt + 5000).toISOString();
  const staleTs = new Date(startedAt - 5000).toISOString();

  it('a server-marked frame is replayed even with a post-start timestamp (marker beats epoch)', () => {
    expect(frameReplayed({ replayed: true, timestamp: liveTs }, startedAt)).toBe(true);
  });

  it('a server-marked frame is replayed even without a timestamp', () => {
    expect(frameReplayed({ replayed: true }, startedAt)).toBe(true);
  });

  it('an unmarked frame falls back to the epoch heuristic (older daemons)', () => {
    expect(frameReplayed({ timestamp: staleTs }, startedAt)).toBe(true);
    expect(frameReplayed({ timestamp: liveTs }, startedAt)).toBe(false);
    expect(frameReplayed({}, startedAt)).toBe(false); // untimestamped = live
  });

  it('the marker is one-directional: an explicit replayed:false does NOT overrule the epoch guard', () => {
    // The server never sends replayed:false today; if one ever appeared it
    // would only mean "live at send time", which says nothing about
    // client-side staleness — the epoch fallback must still apply.
    expect(frameReplayed({ replayed: false, timestamp: staleTs }, startedAt)).toBe(true);
    expect(frameReplayed({ replayed: false, timestamp: liveTs }, startedAt)).toBe(false);
  });

  it('a non-boolean marker is ignored (falls back to the epoch heuristic)', () => {
    expect(frameReplayed({ replayed: 'yes', timestamp: liveTs }, startedAt)).toBe(false);
    expect(frameReplayed({ replayed: 1, timestamp: staleTs }, startedAt)).toBe(true);
  });

  it('non-object frames classify as live (no timestamp to compare)', () => {
    expect(frameReplayed(null, startedAt)).toBe(false);
    expect(frameReplayed('frame', startedAt)).toBe(false);
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

  it('honors the server-side replay marker over a live timestamp', () => {
    // A daemon-marked buffer re-delivery of a frame emitted while this client
    // was disconnected: stamped after the wire started, yet still history.
    const raw = JSON.stringify({
      type: 'memory_added',
      timestamp: new Date(startedAt + 60_000).toISOString(),
      replayed: true,
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
