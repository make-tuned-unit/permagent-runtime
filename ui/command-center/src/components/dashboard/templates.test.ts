import { describe, expect, it } from 'vitest';
import { DASHBOARD_TEMPLATES, realizedLayout, comingSoonCards } from './templates';
import { countHoles } from './useLayout';

describe('DASHBOARD_TEMPLATES', () => {
  it('has exactly the five templates from the research doc', () => {
    expect(DASHBOARD_TEMPLATES.map(t => t.id)).toEqual([
      'morning_briefing',
      'operator_command_center',
      'weekly_story',
      'relationships_commitments',
      'growth_and_money',
    ]);
  });

  it('every template has a name, a question, and at least one card', () => {
    for (const t of DASHBOARD_TEMPLATES) {
      expect(t.name.length).toBeGreaterThan(0);
      expect(t.question.length).toBeGreaterThan(0);
      expect(t.cards.length).toBeGreaterThan(0);
    }
  });

  it('no card is wider than the 12-column grid or positioned past it', () => {
    for (const t of DASHBOARD_TEMPLATES) {
      for (const c of t.cards) {
        expect(c.size.w).toBeGreaterThanOrEqual(1);
        expect(c.size.w).toBeLessThanOrEqual(12);
        expect(c.position.x + c.size.w).toBeLessThanOrEqual(12);
      }
    }
  });
});

describe('realizedLayout', () => {
  it("drops a template's comingSoon cards", () => {
    const weeklyStory = DASHBOARD_TEMPLATES.find(t => t.id === 'weekly_story')!;
    const layout = realizedLayout(weeklyStory);
    expect(layout.cards.some(c => c.type === 'week_headline')).toBe(false);
    // Real cards on that template survive.
    expect(layout.cards.some(c => c.type === 'timeline')).toBe(true);
    expect(layout.cards.some(c => c.type === 'council')).toBe(true);
  });

  it('re-packs so dropping a comingSoon card leaves no interior hole', () => {
    const weeklyStory = DASHBOARD_TEMPLATES.find(t => t.id === 'weekly_story')!;
    const layout = realizedLayout(weeklyStory);
    const cells = layout.cards.map(c => ({ id: c.id, x: c.position.x, y: c.position.y, w: c.size.w, h: c.size.h }));
    // A dense re-pack of only the real cards should be gap-free start to end.
    expect(countHoles(cells)).toBe(0);
  });

  it("keeps every card for a template with nothing coming soon (Morning Briefing)", () => {
    const morning = DASHBOARD_TEMPLATES.find(t => t.id === 'morning_briefing')!;
    const layout = realizedLayout(morning);
    expect(layout.cards.length).toBe(morning.cards.length);
  });
});

describe('comingSoonCards', () => {
  it('is empty for a fully-built template', () => {
    const morning = DASHBOARD_TEMPLATES.find(t => t.id === 'morning_briefing')!;
    expect(comingSoonCards(morning)).toEqual([]);
    const operator = DASHBOARD_TEMPLATES.find(t => t.id === 'operator_command_center')!;
    expect(comingSoonCards(operator)).toEqual([]);
  });

  it('names every unbuilt card for a Phase-2-heavy template', () => {
    const relationships = DASHBOARD_TEMPLATES.find(t => t.id === 'relationships_commitments')!;
    const names = comingSoonCards(relationships);
    expect(names).toContain('people_attention');
    expect(names).toContain('open_commitments');
    // `calendar` already exists — must not be listed as coming soon.
    expect(names).not.toContain('calendar');
  });
});
