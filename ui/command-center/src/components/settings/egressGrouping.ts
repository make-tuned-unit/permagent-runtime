/**
 * Pure helpers for the Privacy → Egress audit table.
 *
 * Split out of `SettingsView.tsx` so the grouping/summary logic — the part an
 * operator's trust in the audit log actually rests on — has its own unit
 * tests instead of only being exercised through a rendered table.
 */

import type { EgressLogEntry } from '../../lib/api';

/** A run of consecutive, newest-first log entries that share provider, model,
 *  kind and blocked outcome. A burst of N identical calls (the common case for
 *  a chatty session) reads as one row with a count and a time span instead of
 *  N visually identical rows — Little Snitch groups its connection log the
 *  same way: show the TRAFFIC, not one line per packet. Grouping is a RUN, not
 *  a tally: an unrelated call in between starts a fresh group, so two bursts of
 *  the same shape a day apart stay two rows, each honestly time-stamped. */
export interface EgressGroup {
  /** Newest entry in the run (the log is newest-first, so this is first in). */
  newest: EgressLogEntry;
  /** Oldest entry in the run — the far end of the time span. */
  oldest: EgressLogEntry;
  count: number;
  /** Every session id seen in the run, in first-seen order, deduped — "what
   *  caused this" (UX_STANDARD.md R5: show why). */
  sessionIds: string[];
}

export function collapseConsecutiveEgress(log: readonly EgressLogEntry[]): EgressGroup[] {
  const groups: EgressGroup[] = [];
  for (const entry of log) {
    const prev = groups[groups.length - 1];
    const sameRun = prev
      && prev.oldest.provider === entry.provider
      && prev.oldest.model === entry.model
      && prev.oldest.kind === entry.kind
      && prev.oldest.blocked === entry.blocked;
    if (sameRun && prev) {
      prev.oldest = entry;
      prev.count += 1;
      if (entry.sessionId && !prev.sessionIds.includes(entry.sessionId)) prev.sessionIds.push(entry.sessionId);
    } else {
      groups.push({
        newest: entry,
        oldest: entry,
        count: 1,
        sessionIds: entry.sessionId ? [entry.sessionId] : [],
      });
    }
  }
  return groups;
}

/** One row of the egress-audit summary header: how many calls a provider+kind
 *  pair made in the last 24h, and how many of those were blocked. */
export interface EgressSummaryRow {
  provider: string;
  kind: string;
  count: number;
  blocked: number;
}

const DAY_MS = 24 * 60 * 60 * 1000;

/** Per-provider, per-kind call counts within the last 24h (relative to `now`,
 *  injectable for tests), sorted most-called first. An entry whose timestamp
 *  fails to parse is kept rather than silently dropped — a log the audit
 *  cannot date is a fact worth showing, not a fact to hide by filtering it out. */
export function summarizeEgress24h(log: readonly EgressLogEntry[], now: number = Date.now()): EgressSummaryRow[] {
  const cutoff = now - DAY_MS;
  const byKey = new Map<string, EgressSummaryRow>();
  for (const entry of log) {
    const ts = Date.parse(entry.ts);
    if (Number.isFinite(ts) && ts < cutoff) continue;
    const key = `${entry.provider}\u0000${entry.kind}`;
    const row = byKey.get(key) ?? { provider: entry.provider, kind: entry.kind, count: 0, blocked: 0 };
    row.count += 1;
    if (entry.blocked) row.blocked += 1;
    byKey.set(key, row);
  }
  return [...byKey.values()].sort((a, b) => b.count - a.count);
}

/** Hourly call-volume buckets over the last 24h, oldest first — the small
 *  traffic timeline beside the summary header (Little Snitch: show the
 *  traffic). 24 buckets, index 0 is `now - 24h`..`now - 23h`, index 23 is the
 *  current hour. Blocked calls are counted separately so a spike can be read
 *  as "busy" vs "under attack" at a glance. */
export interface EgressHourBucket {
  /** Start of the bucket, epoch ms. */
  start: number;
  allowed: number;
  blocked: number;
}

export function bucketEgressHourly(log: readonly EgressLogEntry[], now: number = Date.now()): EgressHourBucket[] {
  const hourMs = 60 * 60 * 1000;
  const bucketStart = Math.floor(now / hourMs) * hourMs;
  const buckets: EgressHourBucket[] = Array.from({ length: 24 }, (_, i) => ({
    start: bucketStart - (23 - i) * hourMs,
    allowed: 0,
    blocked: 0,
  }));
  for (const entry of log) {
    const ts = Date.parse(entry.ts);
    if (!Number.isFinite(ts)) continue;
    const idx = Math.floor((ts - buckets[0].start) / hourMs);
    if (idx < 0 || idx >= buckets.length) continue;
    if (entry.blocked) buckets[idx].blocked += 1;
    else buckets[idx].allowed += 1;
  }
  return buckets;
}
