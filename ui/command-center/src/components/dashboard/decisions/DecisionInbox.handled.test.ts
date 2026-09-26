import { describe, expect, it } from 'vitest';
import { handledHistoryFilter, handledLinkLabel, handledSentence, handledSummary } from './DecisionInbox.handled';

const now = new Date(2026, 8, 25, 8, 30);

describe('handled link fallback (U18)', () => {
  it('names the daemon window when it sends one, and opens the agent history for it', () => {
    const since = new Date(2026, 8, 24, 21, 0).toISOString();
    const s = handledSummary({ handled_count: 99, handled: { count: 4, since } }, now);
    expect(handledLinkLabel(s)).toBe('4 handled since yesterday 9 PM');
    expect(handledSentence(s, 'Nova')).toBe('Nova handled 4 items since yesterday 9 PM.');
    expect(handledHistoryFilter(s)).toBe('agent');
    expect(s.sinceIso).toBe(since);
    const today = handledSummary({ handled_count: 0, handled: { count: 2, since: new Date(2026, 8, 25, 6, 15).toISOString() } }, now);
    expect(handledLinkLabel(today)).toBe('2 handled since 6:15 AM');
  });

  // Was: "7 handled" plus a sentence crediting the agent, while the link
  // opened the agent-only history. The legacy count is every resolved
  // decision by anyone, so it must neither credit the agent nor open its view.
  it('without a window the any-actor total reads "N resolved", credits nobody and opens everything', () => {
    const s = handledSummary({ handled_count: 7 }, now);
    expect(handledLinkLabel(s)).toBe('7 resolved');
    expect(handledSentence(s, 'Nova')).toBeNull();
    expect(handledHistoryFilter(s)).toBe('all');
    expect(handledLinkLabel(s)).not.toMatch(/overnight|since/);
  });

  it('an agent window with nothing in it opens everything', () => {
    const s = handledSummary({ handled_count: 7, handled: { count: 0, since: new Date(2026, 8, 25, 6, 0).toISOString() } }, now);
    expect(handledLinkLabel(s)).toBe('History');
    expect(handledHistoryFilter(s)).toBe('all');
  });

  it('reads "History" at zero', () => {
    expect(handledLinkLabel(handledSummary({ handled_count: 0 }, now))).toBe('History');
    expect(handledLinkLabel(handledSummary(null, now))).toBe('History');
  });
});
