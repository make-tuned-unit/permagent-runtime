import { describe, it, expect } from 'vitest';
import type { EgressLogEntry } from '../../lib/api';
import { bucketEgressHourly, collapseConsecutiveEgress, summarizeEgress24h } from './egressGrouping';

function entry(overrides: Partial<EgressLogEntry>): EgressLogEntry {
  return {
    id: overrides.id ?? Math.random().toString(36).slice(2),
    ts: overrides.ts ?? new Date().toISOString(),
    provider: overrides.provider ?? 'anthropic',
    model: overrides.model ?? 'claude-sonnet-5',
    sessionId: overrides.sessionId ?? null,
    projectId: overrides.projectId ?? null,
    kind: overrides.kind ?? 'chat',
    blocked: overrides.blocked ?? false,
    contentHash: overrides.contentHash ?? 'abc123',
    prompt: overrides.prompt ?? null,
  };
}

describe('collapseConsecutiveEgress', () => {
  it('collapses a consecutive run of identical calls into one group with a count', () => {
    const log = [
      entry({ id: '3', ts: '2026-09-25T10:02:00Z' }),
      entry({ id: '2', ts: '2026-09-25T10:01:00Z' }),
      entry({ id: '1', ts: '2026-09-25T10:00:00Z' }),
    ];
    const groups = collapseConsecutiveEgress(log);
    expect(groups).toHaveLength(1);
    expect(groups[0].count).toBe(3);
    expect(groups[0].newest.id).toBe('3');
    expect(groups[0].oldest.id).toBe('1');
  });

  it('does not merge an unrelated call in between — a run, not a tally', () => {
    const log = [
      entry({ id: '3', model: 'claude-sonnet-5' }),
      entry({ id: '2', model: 'gpt-5' }),
      entry({ id: '1', model: 'claude-sonnet-5' }),
    ];
    const groups = collapseConsecutiveEgress(log);
    expect(groups).toHaveLength(3);
    expect(groups.every(g => g.count === 1)).toBe(true);
  });

  it('separates groups by provider, kind, and blocked outcome even when adjacent', () => {
    const log = [
      entry({ id: '4', blocked: true }),
      entry({ id: '3', blocked: false }),
      entry({ id: '2', kind: 'embedding' }),
      entry({ id: '1', kind: 'chat' }),
    ];
    const groups = collapseConsecutiveEgress(log);
    expect(groups).toHaveLength(4);
  });

  it('collects every distinct session id in the run, in first-seen order', () => {
    const log = [
      entry({ id: '3', sessionId: 'sess-b' }),
      entry({ id: '2', sessionId: 'sess-a' }),
      entry({ id: '1', sessionId: 'sess-a' }),
    ];
    const groups = collapseConsecutiveEgress(log);
    expect(groups).toHaveLength(1);
    expect(groups[0].sessionIds).toEqual(['sess-b', 'sess-a']);
  });

  it('returns an empty array for an empty log', () => {
    expect(collapseConsecutiveEgress([])).toEqual([]);
  });
});

describe('summarizeEgress24h', () => {
  const now = new Date('2026-09-25T12:00:00Z').getTime();

  it('counts calls per provider+kind within the last 24h, sorted by volume', () => {
    const log = [
      entry({ provider: 'anthropic', kind: 'chat', ts: '2026-09-25T11:00:00Z' }),
      entry({ provider: 'anthropic', kind: 'chat', ts: '2026-09-25T10:00:00Z' }),
      entry({ provider: 'openai', kind: 'embedding', ts: '2026-09-25T09:00:00Z' }),
    ];
    const rows = summarizeEgress24h(log, now);
    expect(rows).toEqual([
      { provider: 'anthropic', kind: 'chat', count: 2, blocked: 0 },
      { provider: 'openai', kind: 'embedding', count: 1, blocked: 0 },
    ]);
  });

  it('excludes calls older than 24h', () => {
    const log = [
      entry({ ts: '2026-09-24T11:00:00Z' }), // 25h ago
    ];
    expect(summarizeEgress24h(log, now)).toEqual([]);
  });

  it('counts blocked calls within their provider+kind row', () => {
    const log = [
      entry({ provider: 'anthropic', kind: 'chat', blocked: true, ts: '2026-09-25T11:00:00Z' }),
      entry({ provider: 'anthropic', kind: 'chat', blocked: false, ts: '2026-09-25T11:30:00Z' }),
    ];
    const rows = summarizeEgress24h(log, now);
    expect(rows).toEqual([{ provider: 'anthropic', kind: 'chat', count: 2, blocked: 1 }]);
  });

  it('keeps an entry with an unparseable timestamp rather than dropping it', () => {
    const log = [entry({ ts: 'not-a-date' })];
    expect(summarizeEgress24h(log, now)).toEqual([{ provider: 'anthropic', kind: 'chat', count: 1, blocked: 0 }]);
  });
});

describe('bucketEgressHourly', () => {
  const now = new Date('2026-09-25T12:30:00Z').getTime();

  it('produces 24 hourly buckets ending at the current hour', () => {
    const buckets = bucketEgressHourly([], now);
    expect(buckets).toHaveLength(24);
    expect(new Date(buckets[23].start).toISOString()).toBe('2026-09-25T12:00:00.000Z');
    expect(new Date(buckets[0].start).toISOString()).toBe('2026-09-24T13:00:00.000Z');
  });

  it('sorts a call into its hour bucket, split by blocked vs allowed', () => {
    const log = [
      entry({ ts: '2026-09-25T12:05:00Z', blocked: false }),
      entry({ ts: '2026-09-25T12:10:00Z', blocked: true }),
      entry({ ts: '2026-09-25T09:00:00Z', blocked: false }),
    ];
    const buckets = bucketEgressHourly(log, now);
    expect(buckets[23]).toMatchObject({ allowed: 1, blocked: 1 });
    expect(buckets[20]).toMatchObject({ allowed: 1, blocked: 0 });
    expect(buckets.reduce((n, b) => n + b.allowed + b.blocked, 0)).toBe(3);
  });

  it('drops a call outside the 24h window', () => {
    const log = [entry({ ts: '2026-09-20T12:00:00Z' })];
    const buckets = bucketEgressHourly(log, now);
    expect(buckets.reduce((n, b) => n + b.allowed + b.blocked, 0)).toBe(0);
  });
});
