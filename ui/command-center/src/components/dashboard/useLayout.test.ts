import { describe, it, expect } from 'vitest';
import {
  compactLayoutPass,
  DEFAULT_LAYOUT,
  ensureCalendarCard,
  ensureGrowthCardTallEnough,
  ensureGrowthResultsCard,
  reflow,
  packDense,
  fillHoles,
  hasInteriorHole,
  justifyRows,
  countHoles,
  layoutForDisplay,
  minSpanForWidth,
  stripRetiredCards,
  type PackedCell,
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

/** Every cell of rows [0, bottom) is covered by exactly one card. */
function holes(cells: PackedCell[], cols = 12): string[] {
  const bottom = cells.reduce((m, c) => Math.max(m, c.y + c.h), 0);
  const owner: (string | null)[][] = Array.from({ length: bottom }, () => new Array(cols).fill(null));
  const overlaps: string[] = [];
  for (const c of cells) {
    for (let r = c.y; r < c.y + c.h; r++) for (let k = c.x; k < c.x + c.w; k++) {
      if (owner[r][k]) overlaps.push(`overlap ${owner[r][k]}/${c.id} @${k},${r}`);
      owner[r][k] = c.id;
    }
  }
  const empty: string[] = [];
  owner.forEach((row, r) => row.forEach((o, k) => { if (!o) empty.push(`${k},${r}`); }));
  return [...overlaps, ...empty];
}

describe('dense dashboard packing (QA 2026-09-24: holes at 1456px)', () => {
  // The walk's board: "Needs you" + "To-dos" on a row, then full-width cards.
  // The shelf packer left 5 empty columns beside them and a hole under the
  // shorter one.
  const walkBoard = [
    card('stats', 5, 3), card('calendar', 7, 4),
    card('decisions', 3, 3), card('todos', 4, 5),
    card('timeline', 12, 5), card('recent', 8, 5), card('growth_results', 12, 6), card('council', 12, 5),
  ];

  it('the old shelf layout had holes; the display layout has none', () => {
    const shelf = walkBoard.map((c, i) => ({ id: c.id, x: 0, y: 0, w: c.size.w, h: c.size.h, i }));
    // Reconstruct the old shelf packing to prove the test catches the bug.
    let x = 0; let y = 0; let rowH = 0;
    for (const c of shelf) {
      if (x + c.w > 12) { x = 0; y += rowH; rowH = 0; }
      c.x = x; c.y = y; x += c.w; rowH = Math.max(rowH, c.h);
    }
    expect(holes(shelf).length).toBeGreaterThan(0);
    expect(holes(layoutForDisplay(walkBoard))).toEqual([]);
  });

  it('has no holes at a narrow (~1100px window) grid width either', () => {
    const minSpan = minSpanForWidth(800, 16);
    expect(minSpan).toBeGreaterThan(3);
    const cells = layoutForDisplay(walkBoard, { minSpan });
    expect(holes(cells)).toEqual([]);
    expect(cells.every(c => c.w >= minSpan)).toBe(true);
  });

  it('back-fills a hole with a later card that fits (grid-auto-flow: dense)', () => {
    const cells = packDense([card('a', 7, 4), card('b', 12, 2), card('c', 5, 2)]);
    expect(cells[2]).toMatchObject({ x: 7, y: 0 });
    expect(cells[1]).toMatchObject({ x: 0, y: 4 });
  });

  it('keeps the Customize order: the earlier card is placed first', () => {
    const ab = packDense([card('a', 6, 2), card('b', 6, 2)]);
    const ba = packDense([card('b', 6, 2), card('a', 6, 2)]);
    expect(ab.find(c => c.id === 'a')!.x).toBe(0);
    expect(ba.find(c => c.id === 'b')!.x).toBe(0);
  });

  it('Customize mode (fill: false) draws the true stored sizes', () => {
    const cells = layoutForDisplay([card('decisions', 3, 3), card('todos', 4, 5), card('timeline', 12, 5)], { fill: false });
    expect(cells.map(c => [c.w, c.h])).toEqual([[3, 3], [4, 5], [12, 5]]);
  });

  it('fillHoles never grows a card below the last row of the board', () => {
    const cells = fillHoles([{ id: 'a', x: 0, y: 0, w: 6, h: 2 }, { id: 'b', x: 6, y: 0, w: 6, h: 4 }]);
    // 'a' is capped at a third of its OWN height (dagK coordinator, 2560px
    // pass): it grows toward 'b' but stops short rather than doubling into a
    // half-empty card — the trailing gutter below it is deliberate, not a bug.
    expect(cells[0]).toMatchObject({ w: 6, h: 3 });
    expect(cells[1]).toMatchObject({ h: 4 });
  });

  it('fillHoles caps stretch at a third of the span, in both axes', () => {
    // Rightward: a 3-wide card next to a 9-tall gap should grow by at most 1
    // column (⌈3/3⌉), not annex the whole row.
    const wide = fillHoles([{ id: 'a', x: 0, y: 0, w: 3, h: 2 }, { id: 'b', x: 3, y: 2, w: 9, h: 2 }]);
    expect(wide[0].w).toBe(4);
    // Downward: a 3-tall card above open rows should grow by at most 1 row.
    const tall = fillHoles([{ id: 'a', x: 0, y: 0, w: 6, h: 3 }, { id: 'b', x: 6, y: 0, w: 6, h: 8 }]);
    expect(tall[0].h).toBe(4);
  });

  it('hasInteriorHole is false for a trailing gutter, true for a boxed hole', () => {
    // Trailing gutter: nothing to the right of the empty cell on its row.
    expect(hasInteriorHole([{ id: 'a', x: 0, y: 0, w: 6, h: 2 }])).toBe(false);
    // Boxed: a card sits to the right of the gap on the same row.
    expect(hasInteriorHole([
      { id: 'a', x: 0, y: 0, w: 4, h: 1 },
      { id: 'b', x: 8, y: 0, w: 4, h: 1 },
    ])).toBe(true);
  });

  it('minSpanForWidth is 1 while the width is unknown and grows as the grid narrows', () => {
    expect(minSpanForWidth(0, 16)).toBe(1);
    expect(minSpanForWidth(1150, 16)).toBeLessThanOrEqual(3);
    expect(minSpanForWidth(800, 16)).toBeGreaterThan(minSpanForWidth(1150, 16));
  });

  it('a boxed-in hole dense packing cannot fill falls back to justified rows', () => {
    // This exact board leaves an enclosed 2-cell hole under "stats" after
    // dense packing + stretching; the display layout must still have none.
    expect(countHoles(fillHoles(packDense(walkBoard)))).toBeGreaterThan(0);
    expect(countHoles(layoutForDisplay(walkBoard))).toBe(0);
  });

  it('justifyRows shares a row\'s spare columns and height, in order', () => {
    const cells = justifyRows([card('decisions', 3, 3), card('todos', 4, 5), card('timeline', 12, 5)]);
    expect(cells[0]).toMatchObject({ x: 0, y: 0, h: 5 });
    expect(cells[1]).toMatchObject({ y: 0, h: 5 });
    expect(cells[0].w + cells[1].w).toBe(12);
    expect(cells[1].x).toBe(cells[0].w);
    expect(cells[2]).toMatchObject({ x: 0, y: 5, w: 12 });
    expect(holes(cells)).toEqual([]);
  });

  it('reflow persists the dense positions', () => {
    const out = reflow([card('a', 7, 4), card('b', 12, 2), card('c', 5, 2)]);
    expect(out[2].position).toEqual({ x: 7, y: 0 });
  });
});
