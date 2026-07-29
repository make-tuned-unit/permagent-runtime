// @vitest-environment jsdom
//
// Regression tests for the speak-replies dedupe contract.
//
// This logic regressed three times in one session (turn-position keys, a
// blanket connect-timer, voice turns bypassing the key), each time surfacing
// as Henry re-speaking his greeting when a window opened. The invariants below
// are the ones that actually broke.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { markReplySpoken, replyDedupeKey, setSpeakReplies } from './speakReplies';

vi.mock('./api', () => ({
  synthesizeVoice: vi.fn(async () => new Blob([''], { type: 'audio/wav' })),
}));

describe('replyDedupeKey', () => {
  it('is stable for identical content — the property window-position keys lacked', () => {
    expect(replyDedupeKey('s1', 'Hello there')).toBe(replyDedupeKey('s1', 'Hello there'));
  });

  it('separates sessions so a switch never suppresses a new session first reply', () => {
    expect(replyDedupeKey('s1', 'Hi')).not.toBe(replyDedupeKey('s2', 'Hi'));
  });

  it('distinguishes different replies', () => {
    expect(replyDedupeKey('s1', 'Hi')).not.toBe(replyDedupeKey('s1', 'Bye'));
  });
});

describe('markReplySpoken', () => {
  beforeEach(() => {
    localStorage.clear();
    setSpeakReplies(false);
  });

  it('records the key so a voice-pipeline turn dedupes a later SSE replay', () => {
    markReplySpoken('s1', 'Lets begin');
    expect(localStorage.getItem('permagent-last-spoken-key')).toBe(
      replyDedupeKey('s1', 'Lets begin'),
    );
  });

  it('ignores empty content (a partial stream frame must not claim the slot)', () => {
    markReplySpoken('s1', '');
    expect(localStorage.getItem('permagent-last-spoken-key')).toBeNull();
  });

  it('advances with each new reply so consecutive distinct turns both speak', () => {
    markReplySpoken('s1', 'first');
    const a = localStorage.getItem('permagent-last-spoken-key');
    markReplySpoken('s1', 'second');
    expect(localStorage.getItem('permagent-last-spoken-key')).not.toBe(a);
  });
});
