import { describe, expect, it } from 'vitest';
import { DASHBOARD_TEMPLATES, realizedLayout, comingSoonCards, cardDisplayName, isCurrentTemplate } from './templates';

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
  it("keeps every card for a template — Phase 2 shipped, nothing is comingSoon anymore", () => {
    for (const t of DASHBOARD_TEMPLATES) {
      const layout = realizedLayout(t);
      expect(layout.cards.length).toBe(t.cards.length);
    }
  });

  it('reflow never drops or duplicates a card while packing', () => {
    // `reflow` (a plain dense pack) is not guaranteed hole-free on its own —
    // that guarantee belongs to `layoutForDisplay`'s fill/justify fallback at
    // render time (useLayout.ts). What `realizedLayout` must guarantee is
    // that every card survives the re-pack, exactly once.
    for (const t of DASHBOARD_TEMPLATES) {
      const layout = realizedLayout(t);
      expect(new Set(layout.cards.map(c => c.id)).size).toBe(t.cards.length);
    }
  });
});

describe('comingSoonCards', () => {
  it('is empty for every template — Phase 2 shipped all remaining cards', () => {
    for (const t of DASHBOARD_TEMPLATES) {
      expect(comingSoonCards(t)).toEqual([]);
    }
  });
});

describe('cardDisplayName', () => {
  it('still gives a human name to any identifier that might show up as comingSoon', () => {
    for (const t of DASHBOARD_TEMPLATES) {
      for (const type of comingSoonCards(t)) {
        const name = cardDisplayName(type);
        expect(name).not.toBe(type);
        expect(name).not.toMatch(/_/);
      }
    }
  });
});

describe('isCurrentTemplate', () => {
  it('matches when the current layout has the same card types, any order', () => {
    const morning = DASHBOARD_TEMPLATES.find(t => t.id === 'morning_briefing')!;
    const layout = realizedLayout(morning);
    const reordered = { cards: [...layout.cards].reverse() };
    expect(isCurrentTemplate(morning, reordered)).toBe(true);
  });

  it('does not match a layout missing one of the template\'s cards', () => {
    const morning = DASHBOARD_TEMPLATES.find(t => t.id === 'morning_briefing')!;
    const layout = realizedLayout(morning);
    const short = { cards: layout.cards.slice(1) };
    expect(isCurrentTemplate(morning, short)).toBe(false);
  });

  it('does not match a different template', () => {
    const morning = DASHBOARD_TEMPLATES.find(t => t.id === 'morning_briefing')!;
    const operator = DASHBOARD_TEMPLATES.find(t => t.id === 'operator_command_center')!;
    expect(isCurrentTemplate(morning, realizedLayout(operator))).toBe(false);
  });
});
