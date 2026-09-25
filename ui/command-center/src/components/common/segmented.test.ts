import { describe, expect, it } from 'vitest';
import { segmentDirection } from './segmented';

describe('segmentDirection (R8 item 5)', () => {
  const keys = ['sessions', 'inbox', 'activity', 'spend'] as const;

  it('is forward when moving right', () => {
    expect(segmentDirection(keys, 'sessions', 'spend')).toBe('forward');
    expect(segmentDirection(keys, 'inbox', 'activity')).toBe('forward');
  });

  it('is backward when moving left', () => {
    expect(segmentDirection(keys, 'spend', 'sessions')).toBe('backward');
    expect(segmentDirection(keys, 'activity', 'inbox')).toBe('backward');
  });

  it('a same-tab no-op reads as forward (no leftward motion)', () => {
    expect(segmentDirection(keys, 'inbox', 'inbox')).toBe('forward');
  });
});
