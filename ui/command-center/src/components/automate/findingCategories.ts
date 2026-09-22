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
  available?: boolean;
  availability_reason?: string;
  protection?: string;
  profiles?: Array<{ status: string; bytes: number }>;
}

/** Unknown and legacy labels never supply deletion authority. */
export function categoryOf(f: FindingLike): CategoryKey {
  return f.category && KNOWN_CATEGORIES.has(f.category)
    ? f.category as CategoryKey : 'review_before_removing';
}

export const REGENERABLE_ACK = 'Include rebuildable caches and build outputs — future builds or downloads may take longer.';
export const REVIEW_ACK = 'Include these reviewed files — they may contain data that cannot be recreated.';

/** Legacy presentation hint retained for callers. A second confirmation is
 * never authority to remove a busy or managed item; current server checks win. */
export function requiresSecondConfirm(f: FindingLike): boolean {
  const c = categoryOf(f);
  return c === 'in_use' || c === 'managed_by_macos';
}

/** Every pending item belongs to exactly one bucket; acknowledgments never
 * bypass the server's current availability veto. */
export function bulkEligible<T extends FindingLike>(
  findings: T[], includeRegenerable: boolean, includeReview = false,
): { eligible: T[]; optional: T[]; excluded: T[] } {
  const eligible: T[] = [];
  const optional: T[] = [];
  const excluded: T[] = [];
  for (const f of findings) {
    if (f.action_taken) continue;
    if (f.available === false || !f.category || !KNOWN_CATEGORIES.has(f.category)) {
      excluded.push(f);
      continue;
    }
    switch (categoryOf(f)) {
      case 'in_use':
      case 'managed_by_macos': excluded.push(f); break;
      case 'regenerable_costly': (includeRegenerable ? eligible : optional).push(f); break;
      case 'review_before_removing': (includeReview ? eligible : optional).push(f); break;
      case 'safe_to_remove': eligible.push(f); break;
    }
  }
  return { eligible, optional, excluded };
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

/** Selected bytes never include busy profiles, unsupported layouts or roots. */
export function cleanupBytes(f: FindingLike): number {
  return f.protection === 'cargo-profile-locks'
    ? (f.profiles || []).filter(p => p.status === 'eligible').reduce((sum, p) => sum + p.bytes, 0)
    : f.size_bytes;
}
