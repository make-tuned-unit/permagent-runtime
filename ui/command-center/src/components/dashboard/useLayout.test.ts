import { describe, it, expect } from 'vitest';
import { normalizeCompactCards, reflow, type DashboardLayoutData } from './useLayout';

function card(type: string, w: number, h: number) {
  return { id: type, type, position: { x: 0, y: 0 }, size: { w, h }, visible: true };
}

describe('normalizeCompactCards', () => {
  it('shrinks weather and system cards still at the old default size', () => {
    const layout: DashboardLayoutData = { cards: [card('weather', 5, 4), card('system_stats', 5, 4)] };
    const { layout: out, changed } = normalizeCompactCards(layout);
    expect(changed).toBe(true);
    expect(out.cards[0].size).toEqual({ w: 3, h: 2 });
    expect(out.cards[1].size).toEqual({ w: 3, h: 2 });
  });

  it('leaves a deliberately resized card alone', () => {
    // Someone who dragged their weather card to 8x6 has said what they want.
    // A "migration" that overrode that would be a bug wearing a helpful hat.
    const layout: DashboardLayoutData = { cards: [card('weather', 8, 6)] };
    const { layout: out, changed } = normalizeCompactCards(layout);
    expect(changed).toBe(false);
    expect(out.cards[0].size).toEqual({ w: 8, h: 6 });
  });

  it('leaves every other card type untouched', () => {
    const layout: DashboardLayoutData = { cards: [card('hero', 5, 4), card('recent', 5, 4)] };
    const { changed } = normalizeCompactCards(layout);
    expect(changed).toBe(false);
  });

  it('returns the identical object when nothing changed, so no needless persist', () => {
    const layout: DashboardLayoutData = { cards: [card('hero', 7, 4)] };
    const { layout: out, changed } = normalizeCompactCards(layout);
    expect(changed).toBe(false);
    expect(out).toBe(layout);
  });

  it('repacks positions after shrinking so no hole is left behind', () => {
    const layout: DashboardLayoutData = {
      cards: [card('weather', 5, 4), card('hero', 7, 4)],
    };
    const { layout: out } = normalizeCompactCards(layout);
    // weather is now 3 wide, so hero starts at x=3 and still fits the row.
    expect(out.cards[0].position).toEqual({ x: 0, y: 0 });
    expect(out.cards[1].position).toEqual({ x: 3, y: 0 });
  });

  it('is idempotent — a second pass reports no change', () => {
    const layout: DashboardLayoutData = { cards: [card('weather', 5, 4)] };
    const once = normalizeCompactCards(layout);
    const twice = normalizeCompactCards(once.layout);
    expect(twice.changed).toBe(false);
  });

  it('handles an empty dashboard', () => {
    const { changed } = normalizeCompactCards({ cards: [] });
    expect(changed).toBe(false);
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
