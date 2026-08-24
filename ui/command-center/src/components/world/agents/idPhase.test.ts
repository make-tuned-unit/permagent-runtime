// idPhase.ts is the fix for "seven agents on one clock breathe in unison" —
// its whole job is to be a stable, well-distributed hash from id to a point on
// the circle. These tests pin exactly that: determinism, range, uniqueness
// across the real roster, and that the seven real ids actually spread out
// rather than landing in a clump (which would silently defeat the fix).

import { describe, expect, it } from 'vitest';
import { getIdPhase, hashId } from './idPhase';
import { ROSTER } from './roster';

describe('hashId', () => {
  it('is a pure function of the string content', () => {
    expect(hashId('henry')).toBe(hashId('henry'));
  });

  it('is sensitive to case and content — different strings hash differently', () => {
    expect(hashId('henry')).not.toBe(hashId('Henry'));
    expect(hashId('henry')).not.toBe(hashId('librarian'));
  });

  it('always returns a non-negative uint32', () => {
    for (const agent of ROSTER) {
      const h = hashId(agent.id);
      expect(Number.isInteger(h)).toBe(true);
      expect(h).toBeGreaterThanOrEqual(0);
      expect(h).toBeLessThanOrEqual(0xffffffff);
    }
  });
});

describe('getIdPhase', () => {
  it('is deterministic — same id, same phase, every call', () => {
    for (const agent of ROSTER) {
      const first = getIdPhase(agent.id);
      const second = getIdPhase(agent.id);
      expect(second).toBe(first);
    }
  });

  it('is stable across "reloads" (repeat calls in a fresh evaluation order still agree)', () => {
    // There is no real process boundary inside a test, but re-deriving the
    // phase from scratch for a shuffled id order and comparing against the
    // roster-order pass is the closest proxy: the value must not depend on
    // call order, iteration count, or anything but the id string itself.
    const rosterOrder = ROSTER.map((a) => getIdPhase(a.id));
    const reversedOrder = [...ROSTER]
      .reverse()
      .map((a) => getIdPhase(a.id))
      .reverse();
    expect(reversedOrder).toEqual(rosterOrder);
  });

  it('returns a value in [0, 2π) for every real roster id', () => {
    expect(ROSTER.length).toBeGreaterThan(0);
    for (const agent of ROSTER) {
      const p = getIdPhase(agent.id);
      expect(p).toBeGreaterThanOrEqual(0);
      expect(p).toBeLessThan(Math.PI * 2);
    }
  });

  it('gives every real roster id a distinct phase', () => {
    const phases = ROSTER.map((a) => getIdPhase(a.id));
    const unique = new Set(phases.map((p) => p.toFixed(9)));
    expect(unique.size).toBe(ROSTER.length);
  });

  it('never rename-normalizes an id — "henry" and "Henry" are different keys', () => {
    // The bible is explicit: `henry` is the stable id, "Aria"-style display
    // names are a different concern entirely. This hash must not special-case
    // or normalize casing/whitespace — it just hashes whatever string it's given.
    expect(getIdPhase('henry')).not.toBe(getIdPhase('Henry'));
  });

  it('spreads the seven real roster ids around the circle instead of clustering', () => {
    // Sort the phases and look at the gaps between consecutive points
    // (wrapping the last gap back to the first). If the hash clustered all
    // seven ids into a small arc, one gap would swallow almost the whole
    // circle. A generous bound (half the circle) is enough to catch that
    // failure mode without being a brittle assertion on the exact hash.
    const phases = [...ROSTER.map((a) => getIdPhase(a.id))].sort((a, b) => a - b);
    const gaps = phases.map((p, i) => {
      const next = i === phases.length - 1 ? phases[0] + Math.PI * 2 : phases[i + 1];
      return next - p;
    });
    const maxGap = Math.max(...gaps);
    expect(maxGap).toBeLessThan(Math.PI);
  });
});
