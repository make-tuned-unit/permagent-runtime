/**
 * Storage-insights bulk-safety rules — pure logic tests (no React).
 *
 * Pins the fallback rule for legacy ledgers and the never-bulk guarantees
 * for `in_use` / `managed_by_macos` findings (see findingCategories.ts for
 * the incident these rules exist to prevent).
 */

import { describe, expect, it } from 'vitest';
import {
  bulkEligible,
  categoryBreakdown,
  categoryOf,
  CATEGORY_LABELS,
  requiresSecondConfirm,
  type FindingLike,
} from './findingCategories';

function finding(overrides: Partial<FindingLike> & Pick<FindingLike, 'id'>): FindingLike {
  return {
    size_bytes: 1000,
    recommendation: 'Review before removing',
    action_taken: null,
    category: null,
    consequence: null,
    ...overrides,
  };
}

describe('categoryOf', () => {
  it('falls back to safe_to_remove for a legacy finding with no category and the exact "Safe to remove" recommendation', () => {
    const f = finding({ id: 'f1', category: undefined, recommendation: 'Safe to remove' });
    expect(categoryOf(f)).toBe('safe_to_remove');
  });

  it('falls back to review_before_removing for a legacy finding with no category and any other recommendation', () => {
    const f = finding({ id: 'f2', category: undefined, recommendation: 'Consider removing' });
    expect(categoryOf(f)).toBe('review_before_removing');
  });

  it('trusts an explicit server category even when it disagrees with the legacy recommendation text', () => {
    const f = finding({ id: 'f3', category: 'in_use', recommendation: 'Safe to remove' });
    expect(categoryOf(f)).toBe('in_use');
  });

  it('treats an unknown/garbled category string as review_before_removing, never as safe', () => {
    const f = finding({ id: 'f4', category: 'totally_made_up', recommendation: 'Safe to remove' });
    expect(categoryOf(f)).toBe('review_before_removing');
  });
});

describe('requiresSecondConfirm', () => {
  it('is true for in_use and managed_by_macos', () => {
    expect(requiresSecondConfirm(finding({ id: 'a', category: 'in_use' }))).toBe(true);
    expect(requiresSecondConfirm(finding({ id: 'b', category: 'managed_by_macos' }))).toBe(true);
  });

  it('is false for safe_to_remove, regenerable_costly, and review_before_removing', () => {
    expect(requiresSecondConfirm(finding({ id: 'c', category: 'safe_to_remove' }))).toBe(false);
    expect(requiresSecondConfirm(finding({ id: 'd', category: 'regenerable_costly' }))).toBe(false);
    expect(requiresSecondConfirm(finding({ id: 'e', category: 'review_before_removing' }))).toBe(false);
  });
});

describe('bulkEligible', () => {
  const pool: FindingLike[] = [
    finding({ id: 'safe-1', category: 'safe_to_remove', size_bytes: 100 }),
    finding({ id: 'safe-2', category: 'safe_to_remove', size_bytes: 200 }),
    finding({ id: 'regen-1', category: 'regenerable_costly', size_bytes: 5000 }),
    finding({ id: 'inuse-1', category: 'in_use', consequence: '5 rustc processes are compiling here', size_bytes: 133_000_000_000 }),
    finding({ id: 'macos-1', category: 'managed_by_macos', consequence: 'macOS maintains this cache', size_bytes: 900 }),
    finding({ id: 'review-1', category: 'review_before_removing', size_bytes: 300 }),
    finding({ id: 'already-done', category: 'safe_to_remove', size_bytes: 1, action_taken: 'trashed' }),
  ];

  it('excludes in_use and managed_by_macos even when includeRegenerable is true', () => {
    const { eligible, excluded } = bulkEligible(pool, true);
    expect(eligible.map(f => f.id)).not.toContain('inuse-1');
    expect(eligible.map(f => f.id)).not.toContain('macos-1');
    expect(excluded.map(f => f.id)).toEqual(expect.arrayContaining(['inuse-1', 'macos-1']));
  });

  it('excludes regenerable by default and includes it once opted in', () => {
    const optedOut = bulkEligible(pool, false);
    expect(optedOut.eligible.map(f => f.id)).not.toContain('regen-1');
    expect(optedOut.excluded.map(f => f.id)).toContain('regen-1');

    const optedIn = bulkEligible(pool, true);
    expect(optedIn.eligible.map(f => f.id)).toContain('regen-1');
    expect(optedIn.excluded.map(f => f.id)).not.toContain('regen-1');
  });

  it('always includes safe_to_remove in eligible and never in excluded', () => {
    const { eligible, excluded } = bulkEligible(pool, false);
    expect(eligible.map(f => f.id)).toEqual(expect.arrayContaining(['safe-1', 'safe-2']));
    expect(excluded.map(f => f.id)).not.toContain('safe-1');
    expect(excluded.map(f => f.id)).not.toContain('safe-2');
  });

  it('places review_before_removing findings in neither list', () => {
    const { eligible, excluded } = bulkEligible(pool, true);
    expect(eligible.map(f => f.id)).not.toContain('review-1');
    expect(excluded.map(f => f.id)).not.toContain('review-1');
  });

  it('drops already-actioned findings from both lists', () => {
    const { eligible, excluded } = bulkEligible(pool, true);
    expect(eligible.map(f => f.id)).not.toContain('already-done');
    expect(excluded.map(f => f.id)).not.toContain('already-done');
  });
});

describe('categoryBreakdown', () => {
  it('totals count and bytes per category and sorts by bytes descending', () => {
    const findings: FindingLike[] = [
      finding({ id: 'a', category: 'safe_to_remove', size_bytes: 100 }),
      finding({ id: 'b', category: 'safe_to_remove', size_bytes: 50 }),
      finding({ id: 'c', category: 'regenerable_costly', size_bytes: 9000 }),
      finding({ id: 'd', category: 'in_use', size_bytes: 500 }),
    ];
    const breakdown = categoryBreakdown(findings);
    expect(breakdown).toEqual([
      { category: 'regenerable_costly', label: CATEGORY_LABELS.regenerable_costly, count: 1, bytes: 9000 },
      { category: 'in_use', label: CATEGORY_LABELS.in_use, count: 1, bytes: 500 },
      { category: 'safe_to_remove', label: CATEGORY_LABELS.safe_to_remove, count: 2, bytes: 150 },
    ]);
  });

  it('includes already-actioned findings in the total (caller decides what subset to pass)', () => {
    const findings: FindingLike[] = [
      finding({ id: 'a', category: 'safe_to_remove', size_bytes: 100, action_taken: 'trashed' }),
    ];
    expect(categoryBreakdown(findings)).toEqual([
      { category: 'safe_to_remove', label: CATEGORY_LABELS.safe_to_remove, count: 1, bytes: 100 },
    ]);
  });
});
