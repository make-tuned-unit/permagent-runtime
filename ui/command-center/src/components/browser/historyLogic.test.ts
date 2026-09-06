import { describe, expect, it } from 'vitest';
import { historySuggestions, recordHistory } from './historyLogic';

describe('browser history suggestions', () => {
  it('merges visits and ranks frequent URLs before recent URLs', () => {
    const now = new Date('2026-09-04T12:00:00Z');
    const one = recordHistory([], 'https://reddit.com/r/example', 'Reddit', now);
    const two = recordHistory(one, 'https://news.example/post', 'News', new Date('2026-09-04T12:01:00Z'));
    const three = recordHistory(two, 'https://reddit.com/r/example', 'Reddit', new Date('2026-09-04T12:02:00Z'));
    expect(three[0]).toMatchObject({ url: 'https://reddit.com/r/example', visitCount: 2 });
    expect(historySuggestions(three, 'reddit')).toHaveLength(1);
  });

  it('never persists non-http schemes', () => {
    expect(recordHistory([], 'javascript:alert(1)', 'unsafe')).toEqual([]);
  });
});
