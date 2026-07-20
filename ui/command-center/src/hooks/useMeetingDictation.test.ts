import { describe, it, expect } from 'vitest';
import {
  GAP_MARKER,
  MEETING_CHUNK_SECONDS,
  formatElapsed,
  joinTranscript,
  meetingNoteTitle,
  shouldFlushChunk,
} from './useMeetingDictation';

describe('shouldFlushChunk', () => {
  it('flushes exactly at the chunk boundary and not before', () => {
    const rate = 16000;
    const boundary = rate * MEETING_CHUNK_SECONDS;
    expect(shouldFlushChunk(boundary - 1, rate)).toBe(false);
    expect(shouldFlushChunk(boundary, rate)).toBe(true);
    expect(shouldFlushChunk(boundary + 4096, rate)).toBe(true);
  });

  it('scales with the real sample rate (48 kHz fallback path)', () => {
    const rate = 48000;
    expect(shouldFlushChunk(rate * MEETING_CHUNK_SECONDS - 1, rate)).toBe(false);
    expect(shouldFlushChunk(rate * MEETING_CHUNK_SECONDS, rate)).toBe(true);
  });

  it('never flushes on a degenerate rate', () => {
    expect(shouldFlushChunk(1_000_000, 0)).toBe(false);
  });
});

describe('joinTranscript', () => {
  it('joins chunks in speech order with single spaces', () => {
    expect(joinTranscript(['first part.', 'second part.'])).toBe('first part. second part.');
  });

  it('drops empty/whitespace chunks (silence) without extra spaces', () => {
    expect(joinTranscript(['', 'hello', '   ', 'world'])).toBe('hello world');
  });

  it('keeps gap markers for failed chunks — never a silent drop', () => {
    expect(joinTranscript(['before', GAP_MARKER, 'after'])).toBe(`before ${GAP_MARKER} after`);
  });

  it('returns empty string for an all-silence recording', () => {
    expect(joinTranscript(['', '  '])).toBe('');
  });
});

describe('meetingNoteTitle', () => {
  it('names the note as a meeting dictation with the start date', () => {
    const title = meetingNoteTitle(new Date(2026, 6, 20, 14, 5));
    expect(title.startsWith('Meeting dictation — ')).toBe(true);
    expect(title).toContain('2026');
  });
});

describe('formatElapsed', () => {
  it('renders m:ss under an hour', () => {
    expect(formatElapsed(0)).toBe('0:00');
    expect(formatElapsed(59)).toBe('0:59');
    expect(formatElapsed(61)).toBe('1:01');
    expect(formatElapsed(600)).toBe('10:00');
  });

  it('renders h:mm:ss at an hour and beyond', () => {
    expect(formatElapsed(3600)).toBe('1:00:00');
    expect(formatElapsed(3661)).toBe('1:01:01');
  });

  it('clamps negatives to zero', () => {
    expect(formatElapsed(-5)).toBe('0:00');
  });
});
