/**
 * Setup-time dev-root selection (MomentCode).
 *
 * The step exists because four features independently guessed `~/dev` and all
 * four failed by finding NOTHING — indistinguishable from a clean machine. The
 * merge rule below is where a re-run of the wizard could undo the user's answer
 * and quietly restore the guess, so it is pinned here.
 */
import { describe, expect, it } from 'vitest';
import { mergeRoots } from './MomentCode';

describe('mergeRoots', () => {
  it('pre-ticks discovery on a first run', () => {
    const r = mergeRoots([], ['/Users/x/Documents/dev']);
    expect(r.candidates).toEqual(['/Users/x/Documents/dev']);
    expect(r.preselected).toEqual(['/Users/x/Documents/dev']);
  });

  it('does not re-tick a root the user previously removed', () => {
    // The user confirmed only ~/code last time; discovery still finds ~/dev.
    // Offering it is right; re-selecting it would silently undo their choice.
    const r = mergeRoots(['/Users/x/code'], ['/Users/x/code', '/Users/x/dev']);
    expect(r.candidates).toEqual(['/Users/x/code', '/Users/x/dev']);
    expect(r.preselected).toEqual(['/Users/x/code']);
  });

  it('lists a confirmed root once, not twice, when discovery finds it too', () => {
    const r = mergeRoots(['/Users/x/dev'], ['/Users/x/dev']);
    expect(r.candidates).toEqual(['/Users/x/dev']);
  });

  it('puts confirmed answers ahead of fresh proposals', () => {
    const r = mergeRoots(['/Users/x/code'], ['/Users/x/dev']);
    expect(r.candidates[0]).toBe('/Users/x/code');
  });

  it('reports nothing to select when neither source has anything', () => {
    const r = mergeRoots([], []);
    expect(r.candidates).toEqual([]);
    expect(r.preselected).toEqual([]);
  });
});
