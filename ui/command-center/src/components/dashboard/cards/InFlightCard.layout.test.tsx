/** @vitest-environment jsdom */
/**
 * Home's In flight card hard-clipped its rows (E2E QA on the installed app,
 * qa-r1-02-inflight-crop.png): titles cut mid-glyph with no ellipsis ("Find
 * and analyz", "Diagnose event"), the "Ready · permag" subtitles likewise, and
 * the second row sliced through its subtitle at the card's bottom edge.
 *
 * Two causes. The list's columns were `minmax(280px, 1fr)`, so in a Home tile
 * narrower than 280px each row overflowed to the right inside a scroller and
 * the title's ellipsis was drawn off-screen; the subtitle and note lines had
 * no ellipsis at all. And the list filled the grid-sized tile with no regard
 * for where a row ends, so the tile's edge cut whichever row straddled it.
 *
 * jsdom has no layout engine, so this cannot measure a clip. It pins the
 * structure that makes the clip impossible: columns never wider than the
 * tile, no horizontal scroll, every text line truncating with an ellipsis,
 * each row's full text in the shared Tooltip (wrapped and width-bounded, so a
 * multi-paragraph hold note is readable), every row reserving the same lines
 * so snapping cannot expose a sliced row, and the list's viewport sized
 * (from measured row boxes, faked here) to end on a row boundary. It does not
 * prove what WebKit paints, nor that the tile's grid height is sensible.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../world/shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Henry' }));
vi.mock('../../mobius/Mobius', () => ({ Mobius: () => <span data-testid="mobius" /> }));
vi.mock('../../../lib/store', () => ({
  useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({ openGoalDetail: () => {} }),
}));

import { InFlightCard, wholeRowsHeight } from './InFlightCard';
import { TOOLTIP_COLD_DELAY_MS, _resetTooltipWarmForTests } from '../../common/Tooltip';
import type { ActiveGoal } from '../../../lib/useLiveGoals';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const GOALS = [
  {
    id: 'g1', project_id: 'p1', state: 'ready', assigned_to: 'permagent-runtime',
    title: 'Find and analyze the slowest daemon routes on the Home screen',
    routing_note: 'Routed to the local tier because the budget for today is nearly spent',
  },
  {
    id: 'g2', project_id: 'p1', state: 'ready', assigned_to: 'permagent-runtime',
    title: 'Diagnose event stream stalls after the laptop wakes from sleep',
  },
] as unknown as ActiveGoal[];

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

const q = (id: string) => container.querySelector(`[data-testid="${id}"]`) as HTMLElement;
const qa = (id: string) => Array.from(container.querySelectorAll(`[data-testid="${id}"]`)) as HTMLElement[];

describe('InFlightCard fits a grid-sized tile', () => {
  it('never lays a column out wider than the tile, and never scrolls sideways', () => {
    act(() => root.render(<InFlightCard goals={GOALS} />));
    const list = q('inflight-list');
    expect(list.style.overflowX).toBe('hidden');
    const grid = list.firstElementChild as HTMLElement;
    // Read the raw attribute: jsdom's CSSOM may not model grid values.
    const style = grid.getAttribute('style') ?? '';
    expect(style).toContain('min(280px, 100%)');
    expect(style).not.toMatch(/minmax\(280px/);
  });

  it('clamps the title to 2 lines and truncates status/note with an ellipsis', () => {
    act(() => root.render(<InFlightCard goals={GOALS} />));
    const texts = qa('inflight-row-text');
    expect(texts).toHaveLength(2);
    // Both rows have title, status and note line (row 2's note line is blank).
    const lines = texts.flatMap(t => Array.from(t.children) as HTMLElement[]);
    expect(lines).toHaveLength(6);
    lines.forEach((line, i) => {
      const label = line.textContent ?? '';
      const isTitle = i % 3 === 0;
      if (isTitle) {
        // Coordinator, 2560px pass: a wide tile has room the title was still
        // truncating with (R3) — the title clamps to 2 lines instead of
        // cutting to 1, status/note stay single-line beneath it.
        expect(line.style.display, label).toBe('-webkit-box');
        expect(line.style.webkitLineClamp, label).toBe('2');
        expect(line.style.overflow, label).toBe('hidden');
      } else {
        expect(line.style.whiteSpace, label).toBe('nowrap');
        expect(line.style.overflow, label).toBe('hidden');
        expect(line.style.textOverflow, label).toBe('ellipsis');
      }
    });
    // The text column may shrink below its content so the ellipsis can engage.
    for (const t of texts) expect(t.style.minWidth).toBe('0');
    expect(lines[1].textContent).toBe('Ready · permagent-runtime');
  });

  it('keeps each row\'s full text reachable in the shared Tooltip, never a native title', () => {
    vi.useFakeTimers();
    try {
      _resetTooltipWarmForTests();
      act(() => root.render(<InFlightCard goals={GOALS} />));
      const row = qa('inflight-row')[0];
      expect(row.hasAttribute('title')).toBe(false);
      act(() => { row.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
      act(() => { vi.advanceTimersByTime(TOOLTIP_COLD_DELAY_MS); });
      const tip = document.querySelector('[role="tooltip"]');
      expect(tip?.textContent).toContain(GOALS[0].title);
      expect(tip?.textContent).toContain('Ready · permagent-runtime');
      expect(tip?.textContent).toContain(GOALS[0].routing_note as string);
      act(() => { row.dispatchEvent(new MouseEvent('mouseout', { bubbles: true })); });
    } finally {
      vi.useRealTimers();
    }
  });

  it('shows a long, multi-line hold note in full, wrapped inside a bounded tip', () => {
    // hold_note is the router's whole last plan with a git status appended
    // (cards.rs HoldState.last_plan), not a label. The shared bubble is nowrap
    // with no width cap, so unbounded it was one line running off the screen.
    const holdNote = [
      'Plan: split the Home poller into one status source and retire the second name.',
      '',
      'Tracked modifications:',
      ' M ui/command-center/src/components/dashboard/cards/InFlightCard.tsx',
      ' M crates/goose/src/cost_router/budget.rs',
      'a'.repeat(300),
    ].join('\n');
    const held = [{ ...GOALS[1], id: 'g3', hold_note: holdNote }] as unknown as ActiveGoal[];
    vi.useFakeTimers();
    try {
      _resetTooltipWarmForTests();
      act(() => root.render(<InFlightCard goals={held} />));
      const row = qa('inflight-row')[0];
      act(() => { row.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
      act(() => { vi.advanceTimersByTime(TOOLTIP_COLD_DELAY_MS); });
      const body = document.querySelector('[data-testid="inflight-row-tip"]') as HTMLElement;
      const noteEl = document.querySelector('[data-testid="inflight-row-tip-note"]') as HTMLElement;
      // Every character of the note is in the tip, line breaks included.
      expect(noteEl.textContent).toBe(holdNote);
      // Kept as lines, not collapsed into one run-on line.
      expect(noteEl.style.whiteSpace).toBe('pre-wrap');
      // The tip's own width is capped and it wraps, overriding the bubble's
      // nowrap, and even an unbroken run (a hash, a path) may break.
      expect(body.style.maxWidth).toBe('360px');
      expect(body.style.whiteSpace).toBe('normal');
      expect(body.getAttribute('style') ?? '').toContain('overflow-wrap: anywhere');
      // The row itself still shows it as one truncated line and says Held.
      const lines = Array.from(qa('inflight-row-text')[0].children) as HTMLElement[];
      expect(lines[1].textContent).toBe('Held · permagent-runtime');
      expect(lines[2].style.whiteSpace).toBe('nowrap');
      act(() => { row.dispatchEvent(new MouseEvent('mouseout', { bubbles: true })); });
    } finally {
      vi.useRealTimers();
    }
  });

  it('gives every row the same lines, so the snap pitch equals the viewport pitch', () => {
    // Row 1 has a note, row 2 none, row 3 a note. If row 2 were a line shorter,
    // snapping to it in a one-row viewport would show row 3's top slice cut
    // mid-glyph. jsdom cannot measure heights; this pins the structure that
    // makes the heights equal: the same three lines with the same styles.
    const mixed = [GOALS[0], GOALS[1], { ...GOALS[0], id: 'g4' }] as unknown as ActiveGoal[];
    act(() => root.render(<InFlightCard goals={mixed} />));
    const notes = qa('inflight-row-note');
    expect(notes).toHaveLength(3);
    const shape = (el: HTMLElement) => [el.style.lineHeight, el.style.marginTop, el.style.fontSize, el.style.whiteSpace].join('|');
    expect(shape(notes[1])).toBe(shape(notes[0]));
    expect(shape(notes[2])).toBe(shape(notes[0]));
    // The blank one still occupies a line (a non-breaking space, not empty),
    // and is hidden from assistive tech.
    expect(notes[1].textContent).toBe('\u00a0');
    expect(notes[1].getAttribute('aria-hidden')).toBe('true');
    expect(notes[0].hasAttribute('aria-hidden')).toBe(false);
    for (const t of qa('inflight-row-text')) expect(t.children).toHaveLength(3);
  });

  it('sizes the list viewport to end on a row boundary, so no row is half-drawn', () => {
    // Fake the layout jsdom cannot do: the tile leaves 150px under the title;
    // row 1 spans 0–96, row 2 starts after the 16px gap at 112 and ends at 208.
    // Only row 1 fits whole, so the viewport must be 96px — not 150px, which
    // is exactly the "second row cut through its subtitle" QA saw.
    vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(function (this: HTMLElement) {
      return this.dataset.testid === 'inflight-region' ? 150 : 0;
    });
    vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockImplementation(function (this: HTMLElement) {
      return this.dataset.testid === 'inflight-row' ? 96 : 0;
    });
    vi.spyOn(HTMLElement.prototype, 'offsetTop', 'get').mockImplementation(function (this: HTMLElement) {
      if (this.dataset.testid !== 'inflight-row') return 0;
      return qa('inflight-row').indexOf(this) * 112;
    });
    act(() => root.render(<InFlightCard goals={GOALS} />));
    const list = q('inflight-list');
    expect(list.style.height).toBe('96px');
    expect(list.style.overflowY).toBe('auto'); // row 2 stays reachable by scrolling
    // And a scrolled list rests on a row start, keeping the viewport whole.
    expect(list.style.scrollSnapType).toBe('y mandatory');
    for (const row of qa('inflight-row')) expect(row.style.scrollSnapAlign).toBe('start');
  });
});

describe('InFlightCard at Home\'s real tile size (~180 x 230)', () => {
  it('gives each row\'s text the row\'s width: no logo beside it', () => {
    // QA (real-size walk): "2 active" over a box showing only the Permagent
    // logo. Mobius `size={36}` is its height; the active frames are 1024x485,
    // so it drew 76px wide and left a ~16px text column in a 180px tile.
    act(() => root.render(<InFlightCard goals={GOALS} />));
    expect(qa('mobius')).toHaveLength(0);
    for (const row of qa('inflight-row')) {
      expect(row.querySelector('img')).toBeNull();
      const inner = row.firstElementChild as HTMLElement;
      // The text block plus the small Auto-approve/Needs review chip
      // (coordinator ruling, dagK) — still no third thing, and specifically
      // no logo: the chip is a `flex-shrink: 0` sibling that never widens
      // past its own label, so the text block still gets the row's width.
      expect(inner.children).toHaveLength(2);
      expect(inner.firstElementChild?.getAttribute('data-testid')).toBe('inflight-row-text');
      expect(inner.children[1].tagName).not.toBe('IMG');
    }
  });

  it('keeps row padding and gap small enough for two rows in a 230px tile', () => {
    // Tile 230 - card padding 2x16 - border 2 - title ~29 leaves ~167px. A row
    // is padding + border + three text lines (~17 + 14 + 2 + 15); with the old
    // 20px padding and 16px gap two rows needed ~208px and only one showed.
    act(() => root.render(<InFlightCard goals={GOALS} />));
    const row = qa('inflight-row')[0];
    expect(row.style.padding).toBe('8px 10px');
    const grid = q('inflight-list').firstElementChild as HTMLElement;
    expect(grid.style.gap).toBe('8px');
  });
});

describe('wholeRowsHeight', () => {
  const rows = [{ top: 0, height: 96 }, { top: 112, height: 96 }, { top: 224, height: 96 }];
  it('ends on the last row that fits whole', () => {
    expect(wholeRowsHeight(150, rows)).toBe(96);
    expect(wholeRowsHeight(208, rows)).toBe(208);
    expect(wholeRowsHeight(319, rows)).toBe(208);
    expect(wholeRowsHeight(1000, rows)).toBe(320);
  });
  it('handles side-by-side columns (same top) as one row', () => {
    const two = [{ top: 0, height: 96 }, { top: 0, height: 96 }, { top: 112, height: 96 }];
    expect(wholeRowsHeight(150, two)).toBe(96);
  });
  it('keeps the full height when not even the first row fits', () => {
    expect(wholeRowsHeight(60, rows)).toBe(60);
    expect(wholeRowsHeight(60, [])).toBe(60);
  });
});

// r3: QA could not confirm the fix because the screen was locked, not because
// the card was unreachable. This pins the two links between this file and
// Home that a silent regression could cut, so "not seen in the app" is never
// a wiring question again: the registry renders THIS component for the
// `in_flight` type, and the default Home layout shows that card. (Whether a
// user's saved layout still has it, and whether any goal is active so rows
// render instead of "Nothing in flight", is live state this cannot see.)
describe('In flight card is wired into Home', () => {
  it('is the component the registry renders, and Home shows it by default', async () => {
    const { CARD_REGISTRY } = await import('./registry');
    const { DEFAULT_LAYOUT } = await import('../useLayout');
    expect(CARD_REGISTRY.in_flight?.component).toBe(InFlightCard);
    const card = DEFAULT_LAYOUT.cards.find(c => c.type === 'in_flight');
    expect(card?.visible).toBe(true);
  });
});
