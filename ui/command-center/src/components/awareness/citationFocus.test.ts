/**
 * Citation → Brain focus mapping. Covers both citation shapes (probed carries
 * id+key, recalled carries id only) and the empty-id/key normalization that lets
 * a cited-but-not-yet-in-graph memory fall through to the preview fallback the
 * shared focusBrainMemory seam (#753) renders.
 */

import { describe, expect, it } from 'vitest';
import { probedFocusTarget, recalledFocusTarget } from './citationFocus';
import type { ProbedMemoryRef, RecalledMemoryRef } from '../../lib/store';

describe('probedFocusTarget', () => {
  it('carries id, key, and a preview (text + the marker score as weight)', () => {
    const m: ProbedMemoryRef = {
      id: 'mem-1', key: 'note:p:1', content_summary: 'The user prefers dark mode', relevance: 0.82, wing: 'work',
    };
    const t = probedFocusTarget(m);
    expect(t.id).toBe('mem-1');
    expect(t.key).toBe('note:p:1');
    expect(t.preview?.text).toBe('The user prefers dark mode');
    expect(t.preview?.weight).toBe(0.82);
  });

  it('normalizes an empty id/key to null so resolution falls through to the preview', () => {
    const t = probedFocusTarget({ id: '', key: '', content_summary: 'x', relevance: 0.1, wing: null });
    expect(t.id).toBeNull();
    expect(t.key).toBeNull();
    expect(t.preview?.text).toBe('x');
  });
});

describe('recalledFocusTarget', () => {
  it('carries id + preview but no key (recalled refs have none)', () => {
    const m: RecalledMemoryRef = { id: 'mem-2', content_summary: 'A recalled fact', signal_score: 0.4 };
    const t = recalledFocusTarget(m);
    expect(t.id).toBe('mem-2');
    expect(t.key).toBeUndefined();
    expect(t.preview?.text).toBe('A recalled fact');
    expect(t.preview?.weight).toBe(0.4);
  });

  it('normalizes an empty id to null', () => {
    const t = recalledFocusTarget({ id: '', content_summary: 'y', signal_score: 0.2 });
    expect(t.id).toBeNull();
    expect(t.preview?.text).toBe('y');
  });
});
