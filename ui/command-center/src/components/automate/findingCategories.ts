/**
 * Storage-insights finding categories — pure decision logic (no React, no
 * fetch), extracted so the bulk-destruction safety rules are unit-testable
 * on their own.
 *
 * Background (why this file exists): a scheduled "storage-insights" recipe
 * labelled 33 findings "Safe to remove" and a bulk "Clean Up All" button
 * trashed all of them in seconds — including a 133 GB cargo target dir with
 * five live rustc builds on it, ~/.cargo/registry, ~/.cache/huggingface,
 * ~/.npm, and this repo's own hermit toolchain cache. The daemon now tags
 * each finding with a `category`; this module enforces what the UI is and
 * is not allowed to do with each category.
 */

export type CategoryKey =
  | 'safe_to_remove'
  | 'regenerable_costly'
  | 'in_use'
  | 'managed_by_macos'
  | 'review_before_removing';

export const CATEGORY_LABELS: Record<CategoryKey, string> = {
  safe_to_remove: 'Safe to remove',
  regenerable_costly: 'Regenerable — costly',
  in_use: 'In use',
  managed_by_macos: 'Managed by macOS',
  review_before_removing: 'Review first',
};

const KNOWN_CATEGORIES: ReadonlySet<string> = new Set(Object.keys(CATEGORY_LABELS));

/** The subset of the daemon's Finding shape this module actually reads. */
export interface FindingLike {
  id: string;
  size_bytes: number;
  recommendation: string;
  action_taken: string | null;
  category?: string | null;
  consequence?: string | null;
}

/**
 * Resolve a finding's category, honoring the server value when present and
 * known. A legacy ledger (predating this fix) omits `category` entirely —
 * for those we trust the old `recommendation` string ONLY for the exact
 * "Safe to remove" match; anything else (including an absent or unrecognized
 * category) defaults to `review_before_removing`, never to a destructive
 * default.
 */
export function categoryOf(f: FindingLike): CategoryKey {
  if (f.category != null) {
    return KNOWN_CATEGORIES.has(f.category) ? (f.category as CategoryKey) : 'review_before_removing';
  }
  return f.recommendation === 'Safe to remove' ? 'safe_to_remove' : 'review_before_removing';
}

/** Individual Trash of this finding must show `consequence` and get a second
 *  explicit confirmation before the request is sent (rule: in_use / managed
 *  _by_macos items are never a one-click delete). */
export function requiresSecondConfirm(f: FindingLike): boolean {
  const c = categoryOf(f);
  return c === 'in_use' || c === 'managed_by_macos';
}

/**
 * Partition PENDING findings (already-actioned findings are dropped
 * entirely — in neither list) into what a bulk action may touch and what it
 * must not.
 *
 * - `in_use` / `managed_by_macos`: always excluded, never opt-in.
 * - `regenerable_costly`: excluded unless `includeRegenerable` is true.
 * - `safe_to_remove`: always eligible.
 * - `review_before_removing`: neither list — bulk action does not touch it
 *   at all (it is not "safe by default" and it is not one of the two
 *   categories with a mandatory excluded-with-consequence callout).
 */
export function bulkEligible<T extends FindingLike>(
  findings: T[],
  includeRegenerable: boolean,
): { eligible: T[]; excluded: T[] } {
  const eligible: T[] = [];
  const excluded: T[] = [];
  for (const f of findings) {
    if (f.action_taken) continue;
    const category = categoryOf(f);
    switch (category) {
      case 'in_use':
      case 'managed_by_macos':
        excluded.push(f);
        break;
      case 'regenerable_costly':
        (includeRegenerable ? eligible : excluded).push(f);
        break;
      case 'safe_to_remove':
        eligible.push(f);
        break;
      case 'review_before_removing':
        // Not part of bulk action, in either direction.
        break;
    }
  }
  return { eligible, excluded };
}

/** Per-category count + byte totals for a set of findings, sorted by bytes
 *  descending (biggest impact first) — used to render the bulk-confirm
 *  breakdown. */
export function categoryBreakdown<T extends FindingLike>(
  findings: T[],
): Array<{ category: CategoryKey; label: string; count: number; bytes: number }> {
  const totals = new Map<CategoryKey, { count: number; bytes: number }>();
  for (const f of findings) {
    const category = categoryOf(f);
    const cur = totals.get(category) ?? { count: 0, bytes: 0 };
    cur.count += 1;
    cur.bytes += f.size_bytes;
    totals.set(category, cur);
  }
  return Array.from(totals.entries())
    .map(([category, { count, bytes }]) => ({ category, label: CATEGORY_LABELS[category], count, bytes }))
    .sort((a, b) => b.bytes - a.bytes);
}
