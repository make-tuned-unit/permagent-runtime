import { describe, expect, it } from 'vitest';
import { handledLinkLabel, handledSentence, handledSummary } from './DecisionInbox.handled';

const now = new Date(2026, 8, 25, 8, 30);

describe('handled link fallback (U18)', () => {
  it('names the daemon window when it sends one', () => {
    const since = new Date(2026, 8, 24, 21, 0).toISOString();
    const s = handledSummary({ handled_count: 99, handled: { count: 4, since } }, now);
    expect(handledLinkLabel(s)).toBe('4 handled since yesterday 9 PM');
    expect(handledSentence(s, 'Henry')).toBe('Henry handled 4 items since yesterday 9 PM.');
    const today = handledSummary({ handled_count: 0, handled: { count: 2, since: new Date(2026, 8, 25, 6, 15).toISOString() } }, now);
    expect(handledLinkLabel(today)).toBe('2 handled since 6:15 AM');
  });

  it('without a window says only "N handled" — never "overnight" or "since"', () => {
    const s = handledSummary({ handled_count: 7 }, now);
    expect(handledLinkLabel(s)).toBe('7 handled');
    expect(handledSentence(s, 'Henry')).not.toMatch(/overnight|since/);
  });

  it('reads "History" at zero', () => {
    expect(handledLinkLabel(handledSummary({ handled_count: 0 }, now))).toBe('History');
    expect(handledLinkLabel(handledSummary(null, now))).toBe('History');
  });
});
