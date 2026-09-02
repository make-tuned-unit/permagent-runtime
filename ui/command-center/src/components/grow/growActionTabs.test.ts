import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  ACTION_CATEGORY_ORDER,
  groupActionsByCategory,
  normalizeActionCategory,
} from './growActionTabs';
import { GROW_SOURCE } from './growSource';

describe('action category tabs', () => {
  it('uses the same keys the generator is allowed to emit', () => {
    const rust = readFileSync(
      join(__dirname, '../../../../../crates/goose-server/src/routes/growth_actions.rs'),
      'utf8',
    );
    const allow = rust.match(
      /category: norm\(\s*item\.get\("category"\),\s*&\[([^\]]+)\]/s,
    );
    expect(allow, 'parse_actions category allowlist not found').toBeTruthy();
    const keys = allow![1]
      .split(',')
      .map((s) => s.trim().replace(/"/g, ''))
      .filter(Boolean);
    // Same set, not necessarily the same order: the tab strip leads with
    // Measurement / Acquisition because those are the moves a review most often
    // repeats, while the parser allowlist is historical.
    expect([...keys].sort()).toEqual([...ACTION_CATEGORY_ORDER].sort());
  });

  it('groups in a stable tab order, not first-seen order', () => {
    const groups = groupActionsByCategory([
      { category: 'aeo', title: 'FAQ' },
      { category: 'measurement', title: 'Events' },
      { category: 'ux', title: 'Hero' },
      { category: 'measurement', title: 'Funnels' },
    ]);
    expect(groups.map((g) => g.key)).toEqual(['measurement', 'ux', 'aeo']);
    expect(groups[0].actions.map((a) => a.title)).toEqual(['Events', 'Funnels']);
  });

  it('does not invent a tab for a garbled category', () => {
    expect(normalizeActionCategory('vibes')).toBe('ux');
    const groups = groupActionsByCategory([{ category: 'vibes', title: 'x' }]);
    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe('ux');
  });

  it('omits empty categories so the strip only shows work', () => {
    const groups = groupActionsByCategory([{ category: 'seo', title: 'meta' }]);
    expect(groups.map((g) => g.key)).toEqual(['seo']);
  });

  it('is what the Actions panel renders', () => {
    const source = GROW_SOURCE;
    expect(source).toContain('groupActionsByCategory');
    expect(source).toContain('aria-label="Action category"');
    expect(source).toContain('showCategory={false}');
  });
});
