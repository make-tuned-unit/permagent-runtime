// Storage-cleanup recovery accounting (issue #242).
//
// The daemon derives the all-time total from every persisted run ledger
// (GET /automation/recovery/total); these helpers keep the UI's period
// labeling honest — a bare "X recovered" that silently means "this run only"
// was the bug.

export interface RecoveryTotals {
  total_recovered_bytes: number;
  runs_with_recovery: number;
  items_trashed: number;
}

export interface RecoveryFinding {
  action_taken: string | null;
  size_recovered_bytes: number | null;
  size_bytes: number;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Bytes recovered by THIS run's trashed findings (kept/skipped/pending = 0). */
export function sumRunRecovered(findings: RecoveryFinding[]): number {
  return findings
    .filter(f => f.action_taken === 'trashed')
    .reduce((s, f) => s + (f.size_recovered_bytes || 0), 0);
}

/** Bytes still awaiting an action in this run. */
export function sumPendingBytes(findings: RecoveryFinding[]): number {
  return findings.filter(f => !f.action_taken).reduce((s, f) => s + f.size_bytes, 0);
}

/**
 * Hero headline. The covered period is ALWAYS explicit — "recovered this run",
 * never a bare "recovered" that reads as a lifetime number (issue #242).
 */
export function recoveryHeadline(
  allActioned: boolean,
  runRecovered: number,
  pendingBytes: number,
): string {
  return allActioned
    ? `${formatBytes(runRecovered)} recovered this run`
    : `${formatBytes(pendingBytes)} to clean up`;
}

/**
 * All-time recovery line for under the hero. Null until totals load, or when
 * nothing has ever been recovered (a "0 B all-time" line is noise on a first
 * scan). Derived server-side from every persisted run ledger, so it survives
 * app restarts.
 */
export function lifetimeRecoveryLine(totals: RecoveryTotals | null): string | null {
  if (!totals || totals.total_recovered_bytes <= 0) return null;
  const items = totals.items_trashed;
  const runs = totals.runs_with_recovery;
  return `${formatBytes(totals.total_recovered_bytes)} recovered all-time — ${items} item${items === 1 ? '' : 's'} across ${runs} run${runs === 1 ? '' : 's'}`;
}
