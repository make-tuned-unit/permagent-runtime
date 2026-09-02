import { describe, it, expect } from 'vitest';
import {
  compactLayoutPass,
  DEFAULT_LAYOUT,
  ensureCalendarCard,
  ensureGrowthCardTallEnough,
  ensureGrowthResultsCard,
  reflow,
  stripRetiredCards,
  type DashboardLayoutData,
} from './useLayout';

function card(type: string, w: number, h: number) {
  return { id: type, type, position: { x: 0, y: 0 }, size: { w, h }, visible: true };
}

describe('compactLayoutPass', () => {
  it('shrinks ambient tiles to their compact ceiling', () => {
    const layout: DashboardLayoutData = { cards: [card('weather', 4, 3), card('system_stats', 5, 4)] };
    const { layout: out, changed } = compactLayoutPass(layout);
    expect(changed).toBe(true);
    expect(out.cards[0].size).toEqual({ w: 3, h: 2 });
    expect(out.cards[1].size).toEqual({ w: 3, h: 2 });
  });

  it('clamps the real-world sizes that the old exact-match rule missed', () => {
    // The previous migration only matched exactly 5x4, so a dashboard
    // auto-arranged to 4x3 / 4x6 shrank nothing and the change was invisible.
    const layout: DashboardLayoutData = {
      cards: [
        card('hero', 4, 3), card('weather', 4, 3), card('system_stats', 4, 3),
        card('decisions', 4, 6), card('stats', 4, 6), card('in_flight', 4, 6),
        card('recent', 8, 6), card('calendar', 4, 6),
      ],
    };
    const { layout: out, changed } = compactLayoutPass(layout);
    expect(changed).toBe(true);
    const byType = Object.fromEntries(out.cards.map(c => [c.type, c.size]));
    expect(byType.weather).toEqual({ w: 3, h: 2 });
    expect(byType.system_stats).toEqual({ w: 3, h: 2 });
    expect(byType.decisions).toEqual({ w: 3, h: 3 });
    expect(byType.stats).toEqual({ w: 4, h: 3 });
    expect(byType.in_flight).toEqual({ w: 4, h: 3 });
    expect(byType.recent).toEqual({ w: 8, h: 5 });
    expect(byType.calendar).toEqual({ w: 4, h: 5 });
  });

  it('never GROWS a card the user made smaller', () => {
    const layout: DashboardLayoutData = { cards: [card('recent', 3, 2)] };
    const { layout: out, changed } = compactLayoutPass(layout);
    expect(changed).toBe(false);
    expect(out.cards[0].size).toEqual({ w: 3, h: 2 });
  });

  it('leaves unknown card types alone', () => {
    const layout: DashboardLayoutData = { cards: [card('some_skill_card', 12, 8)] };
    const { changed } = compactLayoutPass(layout);
    expect(changed).toBe(false);
  });

  it('returns the identical object when nothing changed, so no needless persist', () => {
    const layout: DashboardLayoutData = { cards: [card('weather', 3, 2)] };
    const { layout: out, changed } = compactLayoutPass(layout);
    expect(changed).toBe(false);
    expect(out).toBe(layout);
  });

  it('repacks positions so shrinking leaves no hole', () => {
    const layout: DashboardLayoutData = { cards: [card('weather', 5, 4), card('hero', 6, 3)] };
    const { layout: out } = compactLayoutPass(layout);
    expect(out.cards[0].position).toEqual({ x: 0, y: 0 });
    expect(out.cards[1].position).toEqual({ x: 3, y: 0 });
  });

  it('is idempotent', () => {
    const layout: DashboardLayoutData = { cards: [card('weather', 5, 4)] };
    const once = compactLayoutPass(layout);
    expect(compactLayoutPass(once.layout).changed).toBe(false);
  });

  it('handles an empty dashboard', () => {
    expect(compactLayoutPass({ cards: [] }).changed).toBe(false);
  });
});

describe('ensureCalendarCard', () => {
  it('inserts a calendar card when the layout has none', () => {
    const { layout, changed } = ensureCalendarCard({ cards: [card('hero', 7, 4)] });
    expect(changed).toBe(true);
    expect(layout.cards.some(c => c.type === 'calendar')).toBe(true);
  });

  it('leaves a layout that already has calendar alone', () => {
    const input: DashboardLayoutData = { cards: [card('calendar', 5, 4)] };
    const { layout, changed } = ensureCalendarCard(input);
    expect(changed).toBe(false);
    expect(layout).toBe(input);
  });
});

describe('reflow', () => {
  it('wraps to a new row when a card would overflow 12 columns', () => {
    const out = reflow([card('a', 7, 4), card('b', 7, 3)]);
    expect(out[0].position).toEqual({ x: 0, y: 0 });
    expect(out[1].position).toEqual({ x: 0, y: 4 });
  });

  it('packs cards side by side while they fit', () => {
    const out = reflow([card('a', 3, 2), card('b', 3, 2), card('c', 3, 2)]);
    expect(out.map(c => c.position.x)).toEqual([0, 3, 6]);
    expect(out.every(c => c.position.y === 0)).toBe(true);
  });

  it('advances by the tallest card in the row, not the last one', () => {
    const out = reflow([card('tall', 6, 5), card('short', 6, 2), card('next', 12, 2)]);
    expect(out[2].position).toEqual({ x: 0, y: 5 });
  });

  it('does not mutate its input', () => {
    const input = [card('a', 3, 2)];
    const snapshot = JSON.parse(JSON.stringify(input));
    reflow(input);
    expect(input).toEqual(snapshot);
  });
});

describe('ensureGrowthResultsCard', () => {
  it('adds a growth card when the dashboard does not have one', () => {
    const layout: DashboardLayoutData = { cards: [card('hero', 7, 4)] };
    const { layout: out, changed } = ensureGrowthResultsCard(layout);
    expect(changed).toBe(true);
    expect(out.cards.some((c) => c.type === 'growth_results')).toBe(true);
  });

  it('is a no-op when the card is already present', () => {
    const layout: DashboardLayoutData = { cards: [card('growth_results', 12, 6)] };
    const { layout: out, changed } = ensureGrowthResultsCard(layout);
    expect(changed).toBe(false);
    expect(out).toBe(layout);
  });
});

describe('stripRetiredCards', () => {
  it('removes a persisted hero card instead of leaving a MissingCard ghost tile', () => {
    const layout: DashboardLayoutData = { cards: [card('hero', 7, 4), card('stats', 5, 4)] };
    const { layout: out, changed } = stripRetiredCards(layout);
    expect(changed).toBe(true);
    expect(out.cards.some(c => c.type === 'hero')).toBe(false);
    expect(out.cards.some(c => c.type === 'stats')).toBe(true);
  });

  it('loads a hero-only persisted layout down to nothing without crashing', () => {
    const layout: DashboardLayoutData = { cards: [card('hero', 7, 4)] };
    const { layout: out, changed } = stripRetiredCards(layout);
    expect(changed).toBe(true);
    expect(out.cards).toEqual([]);
  });

  it('is a no-op, same-object passthrough for a layout with no retired cards', () => {
    const layout: DashboardLayoutData = { cards: [card('stats', 5, 4)] };
    const { layout: out, changed } = stripRetiredCards(layout);
    expect(changed).toBe(false);
    expect(out).toBe(layout);
  });

  it('the shipped default layout no longer names the retired hero card', () => {
    expect(DEFAULT_LAYOUT.cards.some(c => c.type === 'hero')).toBe(false);
  });
});

describe('ensureGrowthCardTallEnough', () => {
  it('grows the original 12×4 default to 12×6', () => {
    const layout: DashboardLayoutData = { cards: [card('growth_results', 12, 4)] };
    const { layout: out, changed } = ensureGrowthCardTallEnough(layout);
    expect(changed).toBe(true);
    expect(out.cards[0].size).toEqual({ w: 12, h: 6 });
  });

  it('leaves a resized card alone', () => {
    const layout: DashboardLayoutData = { cards: [card('growth_results', 12, 8)] };
    const { changed } = ensureGrowthCardTallEnough(layout);
    expect(changed).toBe(false);
  });
});
