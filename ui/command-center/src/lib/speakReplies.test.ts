// @vitest-environment jsdom
//
// Regression tests for the speak-replies dedupe contract.
//
// This logic regressed three times in one session (turn-position keys, a
// blanket connect-timer, voice turns bypassing the key), each time surfacing
// as Henry re-speaking his greeting when a window opened. The invariants below
// are the ones that actually broke.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { hasSpokenKey, markReplySpoken, replyDedupeKey, setSpeakReplies } from './speakReplies';

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
    expect(hasSpokenKey(replyDedupeKey('s1', 'Lets begin'))).toBe(true);
  });

  it('ignores empty content (a partial stream frame must not claim a slot)', () => {
    markReplySpoken('s1', '');
    expect(localStorage.getItem('permagent-last-spoken-key')).toBeNull();
  });

  it('leaves a distinct reply un-deduped so consecutive turns both speak', () => {
    markReplySpoken('s1', 'first');
    expect(hasSpokenKey(replyDedupeKey('s1', 'second'))).toBe(false);
  });

  // THE regression: dedupe was a single localStorage slot, so a later voice
  // turn evicted the session's opening reply and the next history replay
  // re-spoke the greeting. Earlier replies must stay remembered.
  it('keeps earlier replies deduped after newer ones are marked', () => {
    markReplySpoken('s1', 'Lets begin');
    for (const later of ['turn one', 'turn two', 'turn three']) {
      markReplySpoken('s1', later);
    }
    expect(hasSpokenKey(replyDedupeKey('s1', 'Lets begin'))).toBe(true);
  });

  it('upgrades the pre-ring single-key format without re-speaking it', () => {
    localStorage.setItem('permagent-last-spoken-key', replyDedupeKey('s1', 'Lets begin'));
    expect(hasSpokenKey(replyDedupeKey('s1', 'Lets begin'))).toBe(true);
  });

  it('bounds growth, evicting only the oldest', () => {
    for (let i = 0; i < 20; i++) markReplySpoken('s1', `reply ${i}`);
    expect(hasSpokenKey(replyDedupeKey('s1', 'reply 19'))).toBe(true);
    expect(hasSpokenKey(replyDedupeKey('s1', 'reply 0'))).toBe(false);
    expect(JSON.parse(localStorage.getItem('permagent-last-spoken-key')!)).toHaveLength(8);
  });
});
