import { describe, expect, it } from 'vitest';
import { newestPerCardSet, type WatcherInsight } from './ProjectOverview';

/**
 * QA 2026-09-20, Projects → Overview: "From the Watcher" printed three entries
 * that named the same three cards in slightly different words, because the
 * Watcher runs daily and re-reports a situation that has not changed.
 */
const card = (id: string) => ({ id, title: id });

describe('newestPerCardSet', () => {
  const stalled: WatcherInsight[] = [
    { text: 'Three critical pre-launch cards stalled 14+ days.', created_at: '2026-09-20T09:00:00Z', cards: [card('a'), card('b'), card('c')] },
    { text: 'Three prep cards stalled 14+ days.', created_at: '2026-09-19T09:00:00Z', cards: [card('c'), card('a'), card('b')] },
    { text: 'Three pre-launch cards stalled 14+ days.', created_at: '2026-09-18T09:00:00Z', cards: [card('b'), card('c'), card('a')] },
  ];

  it('keeps the newest wording and folds the repeats, whatever order the cards are listed in', () => {
    const kept = newestPerCardSet(stalled);
    expect(kept).toHaveLength(1);
    expect(kept[0].created_at).toBe('2026-09-20T09:00:00Z');
  });

  it('keeps observations about different card sets', () => {
    const kept = newestPerCardSet([
      ...stalled,
      { text: 'The brief is still empty.', created_at: '2026-09-20T10:00:00Z', cards: [card('z')] },
    ]);
    expect(kept.map(i => i.text)).toEqual([
      'The brief is still empty.',
      'Three critical pre-launch cards stalled 14+ days.',
    ]);
  });

  it('never folds entries that name no cards — there is nothing to compare', () => {
    const kept = newestPerCardSet([
      { text: 'Momentum is down this week.', created_at: '2026-09-20T09:00:00Z' },
      { text: 'Two goals have no owner.', created_at: '2026-09-19T09:00:00Z', cards: [] },
    ]);
    expect(kept).toHaveLength(2);
  });
});
