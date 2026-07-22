/**
 * Wizard trait-add validation (MomentMeet). The moment surfaces `reason` inline
 * (role="alert") instead of silently swallowing input, so the accept/reject
 * decision is pinned here: trimming, empty vs. non-empty, length cap, and
 * case-insensitive dedupe (audit #603 finding 5).
 */
import { describe, expect, it } from 'vitest';
import { validateTrait } from './atoms';

describe('validateTrait', () => {
  it('accepts a fresh, non-empty trait', () => {
    expect(validateTrait([], 'precise')).toEqual({ ok: true });
    expect(validateTrait(['precise'], 'curious')).toEqual({ ok: true });
  });

  it('rejects empty / whitespace-only input with a reason', () => {
    expect(validateTrait([], '').ok).toBe(false);
    expect(validateTrait([], '   ').ok).toBe(false);
    expect(validateTrait([], '   ').reason).toBeTruthy();
  });

  it('rejects a case-insensitive duplicate and names it', () => {
    const r = validateTrait(['Precise'], 'precise');
    expect(r.ok).toBe(false);
    expect(r.reason).toContain('precise');
  });

  it('trims before comparing so " precise " is a duplicate', () => {
    expect(validateTrait(['precise'], '  precise  ').ok).toBe(false);
  });

  it('rejects an over-long trait', () => {
    expect(validateTrait([], 'x'.repeat(25)).ok).toBe(false);
    expect(validateTrait([], 'x'.repeat(24)).ok).toBe(true);
  });
});
