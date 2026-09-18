/**
 * `inkOnTrim` — the rule that replaces hand-picking an ink per agent.
 *
 * The identity trim palette is frozen and theme-independent, and it is neither
 * one hue nor one lightness, so no single ink token can serve it. The rule is
 * "ink on identity trim is that trim, taken to ink", and the case for the rule
 * is that it independently reproduces the one value a human already chose by
 * eye — `FinanceView.tsx`'s `FINANCIER_BADGE_INK`.
 *
 * The rest is the gate: every entry in the frozen palette, today and whenever
 * the next agent is added, has to clear AA against its own trim.
 */

import { describe, expect, it } from 'vitest';

import { inkOnTrim, contrastRatio } from './tokens';
import { AGENT_TRIM } from '../components/world/shared/palette';

describe('inkOnTrim', () => {
  it('lands on the ink the Finance lane picked by eye', () => {
    // FinanceView.tsx: `const FINANCIER_BADGE_INK = '#3d2e0a'`, chosen for
    // AGENT_TRIM.financier (#C4A35A) before any rule existed. The formula
    // returns #3d2d0a: identical red and blue, one 255th off in green. That
    // near-agreement is the argument for the formula — a human eye and a hue
    // -preserving darkening independently reached the same ink — and the
    // exact figure is asserted rather than rounded away, so a change to the
    // derivation has to come back through this test.
    const derived = inkOnTrim(AGENT_TRIM.financier);
    expect(derived).toBe('#3d2d0a');
    const channels = (hex: string) =>
      [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
    const [dr, dg, db] = channels(derived);
    const [hr, hg, hb] = channels('#3d2e0a');
    expect([Math.abs(dr - hr), Math.abs(dg - hg), Math.abs(db - hb)]).toEqual([0, 1, 0]);
  });

  it('is legible on every trim in the frozen palette', () => {
    const failures: string[] = [];
    for (const [agent, trim] of Object.entries(AGENT_TRIM)) {
      const ratio = contrastRatio(inkOnTrim(trim), trim);
      if (ratio < 4.5) failures.push(`${agent} (${trim}): ${ratio.toFixed(2)}:1`);
    }
    // AA for normal text. A badge label is small text, so 4.5 is the floor
    // that matters, not 3:1.
    expect(failures, 'every identity trim needs an ink that clears AA').toEqual([]);
  });

  it('flips polarity rather than going illegible on a dark trim', () => {
    // `strix` is a dark oxblood — a 14%-lightness ink on it would be a smear.
    // The same construction runs toward light instead, and the result is still
    // hue-matched rather than a flat white.
    const ink = inkOnTrim(AGENT_TRIM.strix);
    expect(contrastRatio(ink, AGENT_TRIM.strix)).toBeGreaterThanOrEqual(4.5);
    // Light, not dark: a near-white ink's own contrast against black is high.
    expect(contrastRatio(ink, '#000000')).toBeGreaterThan(
      contrastRatio(ink, '#ffffff'),
    );
  });

  it('is deterministic and returns a 6-digit hex', () => {
    for (const trim of Object.values(AGENT_TRIM)) {
      const ink = inkOnTrim(trim);
      expect(ink).toMatch(/^#[0-9a-f]{6}$/);
      expect(inkOnTrim(trim)).toBe(ink);
    }
  });
});

describe('contrastRatio', () => {
  it('agrees with the WCAG reference points', () => {
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 5);
    expect(contrastRatio('#ffffff', '#ffffff')).toBeCloseTo(1, 5);
    // Symmetric — the order of the pair must not change the answer.
    expect(contrastRatio('#04141b', '#00bfef')).toBeCloseTo(
      contrastRatio('#00bfef', '#04141b'), 10,
    );
  });
});
