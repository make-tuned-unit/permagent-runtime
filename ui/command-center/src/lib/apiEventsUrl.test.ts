/**
 * Pins the REAL api.sessionEventsUrl contract (P1): the resume cursor rides a
 * `last_event_id` query param — the exact name the daemon's SSE endpoint
 * parses as its Last-Event-ID header equivalent (EventSource cannot set
 * headers on the store's manual reconnect). The store tests mock this module,
 * so this file imports the real one.
 */

import { describe, expect, it } from 'vitest';
import { api } from './api';

describe('api.sessionEventsUrl', () => {
  it('builds the plain events URL when no cursor is recorded', () => {
    expect(api.sessionEventsUrl('sess-1')).toBe('/sessions/sess-1/events');
    expect(api.sessionEventsUrl('sess-1', null)).toBe('/sessions/sess-1/events');
  });

  it('appends ?last_event_id=<cursor> for a resume', () => {
    expect(api.sessionEventsUrl('sess-1', '42')).toBe('/sessions/sess-1/events?last_event_id=42');
  });

  it('URI-encodes both the session id and the cursor', () => {
    expect(api.sessionEventsUrl('a/b', '4 2')).toBe('/sessions/a%2Fb/events?last_event_id=4%202');
  });
});
