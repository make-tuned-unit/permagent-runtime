/** @vitest-environment jsdom */
/**
 * Home's NEEDS YOU card clipped its own "Review →" line at the bottom edge
 * (found driving the real app, 2026-09-18). The card's height comes from the
 * dashboard grid, the card clips with `overflow: hidden`, and every detail line
 * sat in one unshrinkable column above "Review →" — so when all of them were
 * present, the line that says what tapping does was the one cut off.
 *
 * jsdom has no layout engine, so this cannot measure the clip. It pins the
 * structure that makes the clip impossible: the variable details live in a
 * region that may shrink to nothing and clips itself, each detail line
 * truncates instead of wrapping and never shrinks (so the clip drops whole
 * lines rather than slicing every line's glyphs), each truncated line carries
 * its full text in the shared Tooltip, and "Review →" sits OUTSIDE that region and never
 * shrinks. It does not prove the tile's grid height is sensible, nor what
 * WebKit actually paints.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../decisions/useDecisions', () => ({
  useDecisions: () => ({
    data: {
      total_pending: 3,
      handled_count: 7,
      goals_needing_attention: 2,
      goals_in_flight: 4,
      oldest_pending_at: new Date(Date.now() - 3 * 3600_000).toISOString(),
    },
  }),
}));
vi.mock('../decisions/DecisionInbox', () => ({ DecisionInbox: () => null }));
vi.mock('../../settings/useSettings', () => ({
  usePersona: () => ({ data: { display_name: 'Henry' } }),
}));
vi.mock('../../../lib/store', () => ({
  useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({ pendingDecisionInbox: false, clearPendingDecisionInbox: () => {} }),
}));

import { DecisionsCard } from './DecisionsCard';
import { TOOLTIP_COLD_DELAY_MS, _resetTooltipWarmForTests } from '../../common/Tooltip';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

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
});

/** The body's drawn lines: everything but the zero-height wrap anchor. */
function bodyLines(): HTMLElement[] {
  const body = container.querySelector('[data-testid="decisions-card-body"]') as HTMLElement;
  return (Array.from(body.children) as HTMLElement[]).filter(c => c.getAttribute('aria-hidden') !== 'true');
}

function findReview(): HTMLElement {
  const el = Array.from(container.querySelectorAll('div'))
    .find(d => d.textContent === 'Review →') as HTMLElement | undefined;
  expect(el, 'the card must say what tapping does').toBeTruthy();
  return el!;
}

describe('DecisionsCard fits a grid-sized tile', () => {
  it('keeps "Review →" out of the region that is allowed to clip, and never shrinks it', () => {
    act(() => root.render(<DecisionsCard />));
    const card = container.querySelector('[data-testid="decisions-card"]') as HTMLElement;
    expect(card.style.overflow).toBe('hidden');
    expect(card.style.flexDirection).toBe('column');

    const review = findReview();
    // A direct child of the card — not inside anything that clips before it.
    expect(review.parentElement).toBe(card);
    expect(review.style.flexShrink).toBe('0');

    // Everything else that varies lives in a region that absorbs the squeeze.
    const body = container.querySelector('[data-testid="decisions-card-body"]') as HTMLElement;
    expect(body, 'details must live in a shrinkable region').toBeTruthy();
    expect(body.parentElement).toBe(card);
    expect(body.style.minHeight).toBe('0');
    expect(body.style.overflow).toBe('hidden');
    expect(body.textContent).toContain('Henry needs 3 answers');
    expect(body.textContent).toContain('parked goals need your attention');
    expect(body.textContent).toContain('handled 7 routine items');
    // The region precedes "Review →", so the link is always the last line drawn.
    expect(card.lastElementChild).toBe(review);
  });

  it('truncates each detail line instead of wrapping it taller', () => {
    act(() => root.render(<DecisionsCard />));
    const lines = bodyLines().slice(1); // skip the count
    expect(lines.length).toBeGreaterThanOrEqual(4);
    for (const line of lines) {
      expect(line.style.whiteSpace, line.textContent ?? '').toBe('nowrap');
      expect(line.style.textOverflow).toBe('ellipsis');
      expect(line.style.overflow).toBe('hidden');
    }
  });

  it('never squashes a line: every child of the clipping body refuses to shrink', () => {
    // Each detail line is `overflow: hidden`, i.e. a scroll container, so its
    // automatic min-height is 0. Left at the default `flex-shrink: 1`, a short
    // tile shrinks every line in proportion and cuts the bottom off all their
    // glyphs at once — a clipped "Review →" traded for several half-cut lines.
    // Unshrinkable children keep their full height, so a line that does not
    // fit can only move out whole (next test), never be squashed.
    act(() => root.render(<DecisionsCard />));
    const body = container.querySelector('[data-testid="decisions-card-body"]') as HTMLElement;
    const children = Array.from(body.children) as HTMLElement[];
    expect(children.length).toBeGreaterThanOrEqual(5);
    for (const child of children) {
      expect(child.style.flexShrink, child.textContent ?? '').toBe('0');
    }
  });

  it('drops a line that does not fit whole instead of slicing it at the tile edge', () => {
    // Measured in WebKit across tile heights (2026-09-19): with a plain clip
    // the body's bottom edge fell mid-line at most heights, so the last visible
    // line lost the bottom of its glyphs — "no glyph bottoms cut" held at the
    // grid's own sizes by luck. The body now wraps as a column: a line that
    // does not fit moves to the next column, and because every item is the
    // body's full width that column starts past the clip and is not drawn.
    // A first item never wraps, so a zero-height anchor goes first, letting
    // even the count drop out whole. jsdom cannot lay this out; it pins the
    // CSS that does it. The WebKit sweep is what showed it works.
    act(() => root.render(<DecisionsCard />));
    const body = container.querySelector('[data-testid="decisions-card-body"]') as HTMLElement;
    expect(body.style.flexDirection).toBe('column');
    expect(body.style.flexWrap).toBe('wrap');
    expect(body.style.alignContent).toBe('flex-start');
    const anchor = body.firstElementChild as HTMLElement;
    expect(anchor.getAttribute('aria-hidden')).toBe('true');
    expect(anchor.style.height).toBe('0px');
    expect(anchor.style.width).toBe('100%');
    const lines = bodyLines();
    expect(lines.length).toBeGreaterThanOrEqual(5);
    for (const line of lines) expect(line.style.width, line.textContent ?? '').toBe('100%');
    // The count keeps the ramp's 38px display line, not the inherited 48px
    // that overhung a two-row tile's 47.5px body and would be dropped there.
    expect(lines[0].style.lineHeight).toBe('38px');
  });

  it('keeps the full text of each truncated line reachable on hover', () => {
    // The card's aria-label replaces its child text for assistive tech, and a
    // line cut with an ellipsis hides the rest — so each truncated line must
    // carry its whole text itself. It does so through the shared glass Tooltip,
    // never a native `title=` (retired app-wide; Tooltip.test.tsx guards that).
    // This proves hovering a line opens a tooltip with its full text and wires
    // aria-describedby; it does not prove where WebKit paints the bubble.
    vi.useFakeTimers();
    try {
      _resetTooltipWarmForTests();
      act(() => root.render(<DecisionsCard />));
      const lines = bodyLines().slice(1); // skip the count
      expect(lines.length).toBeGreaterThanOrEqual(4);
      for (const line of lines) {
        const text = (line.textContent ?? '').replace(/ →$/, '');
        expect(line.hasAttribute('title'), `${text}: no native title`).toBe(false);
        act(() => { line.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
        act(() => { vi.advanceTimersByTime(TOOLTIP_COLD_DELAY_MS); });
        const tip = document.querySelector('[role="tooltip"]');
        expect(tip?.textContent, text).toBe(text);
        expect(line.getAttribute('aria-describedby'), text).toBe(tip?.id);
        act(() => { line.dispatchEvent(new MouseEvent('mouseout', { bubbles: true })); });
        expect(document.querySelector('[role="tooltip"]')).toBeNull();
        _resetTooltipWarmForTests();
      }
    } finally {
      vi.useRealTimers();
    }
  });
});
