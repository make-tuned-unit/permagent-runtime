import { describe, expect, it } from 'vitest';
import { DEFAULT_LAYOUT, LEGACY_DEFAULT_LAYOUT, LEGACY_MORNING_LAYOUT, LEGACY_DAEMON_LAYOUTS, isDailyHomeLayout, reflow, upgradeHomeDefault } from './useLayout';
import { DASHBOARD_TEMPLATES, mergeDashboardTemplates, realizedLayout } from './templates';

describe('daily Home default', () => {
  it('has one attention card, two commitments and two activity cards in reading order', () => {
    expect(DEFAULT_LAYOUT.cards.map(c => c.type)).toEqual(['decisions', 'calendar', 'todos', 'in_flight', 'recent']);
    expect(realizedLayout(DASHBOARD_TEMPLATES[0])).toEqual(DEFAULT_LAYOUT);
  });
  it.each([...LEGACY_DAEMON_LAYOUTS, LEGACY_DEFAULT_LAYOUT, LEGACY_MORNING_LAYOUT, { cards: reflow(LEGACY_MORNING_LAYOUT.cards) }])('upgrades an exact shipped layout without changing its input', legacy => {
    const before = JSON.stringify(legacy);
    expect(upgradeHomeDefault(legacy)).toEqual(DEFAULT_LAYOUT);
    expect(JSON.stringify(legacy)).toBe(before);
  });
  it.each(['resize', 'hide', 'rename', 'reorder', 'move', 'extend'])('preserves a %s customization', change => {
    const custom = structuredClone(LEGACY_MORNING_LAYOUT);
    if (change === 'resize') custom.cards[0].size.w = 6;
    if (change === 'hide') custom.cards[0].visible = false;
    if (change === 'rename') custom.cards[0].id = 'custom-id';
    if (change === 'reorder') custom.cards.reverse();
    if (change === 'move') custom.cards[0].position.y = 1;
    if (change === 'extend') custom.cards.push({ ...custom.cards[0], id: 'extra' });
    expect(upgradeHomeDefault(custom)).toBe(custom);
    expect(isDailyHomeLayout(custom)).toBe(false);
  });
  it('keeps the new daily template against an older daemon while retaining remote templates', () => {
    const older = { ...DASHBOARD_TEMPLATES[0], cards: LEGACY_MORNING_LAYOUT.cards };
    const remote = { id: 'remote', name: 'Custom template', question: 'What next?', cards: [] };
    expect(mergeDashboardTemplates([older, remote])).toEqual([DASHBOARD_TEMPLATES[0], remote]);
  });
});
