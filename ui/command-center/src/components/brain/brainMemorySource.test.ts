import { describe, expect, it } from 'vitest';
import { humanizeMemorySource, isForgettableMemoryKey } from './brainMemorySource';

describe('humanizeMemorySource', () => {
  it('maps a known RememberOpts tag to its phrase', () => {
    expect(humanizeMemorySource('chat')).toEqual({ phrase: 'from a conversation', known: true });
    expect(humanizeMemorySource('permagent.goal')).toEqual({ phrase: 'from a goal', known: true });
    expect(humanizeMemorySource('permagent.decision.asked')).toEqual({ phrase: 'from a decision', known: true });
    expect(humanizeMemorySource('permagent.card')).toEqual({ phrase: 'from a task', known: true });
  });

  it('is honest about an absent source instead of guessing', () => {
    expect(humanizeMemorySource(null)).toEqual({ phrase: 'source unknown', known: false });
    expect(humanizeMemorySource(undefined)).toEqual({ phrase: 'source unknown', known: false });
    expect(humanizeMemorySource('')).toEqual({ phrase: 'source unknown', known: false });
  });

  it('humanizes an unmapped tag rather than rendering the raw identifier (R3)', () => {
    const { phrase, known } = humanizeMemorySource('permagent.new_thing');
    expect(known).toBe(true);
    expect(phrase).toBe('from new thing');
    expect(phrase).not.toMatch(/permagent\.|_/);
  });
});

describe('isForgettableMemoryKey', () => {
  it('accepts a real memory key', () => {
    expect(isForgettableMemoryKey('note:abc123')).toBe(true);
    expect(isForgettableMemoryKey('chat-session-42-3')).toBe(true);
  });

  it('rejects a missing key', () => {
    expect(isForgettableMemoryKey(null)).toBe(false);
    expect(isForgettableMemoryKey(undefined)).toBe(false);
    expect(isForgettableMemoryKey('')).toBe(false);
  });

  it('rejects the search bar\'s synthetic chat-hit key (colon, not the real hyphenated key)', () => {
    expect(isForgettableMemoryKey('chat:session-42')).toBe(false);
  });
});
