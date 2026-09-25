import { useState, useEffect, useCallback } from 'react';
import { apiFetch } from '../../lib/api';

export interface CardPosition { x: number; y: number }
export interface CardSize { w: number; h: number }
export interface DashboardCardLayout {
  id: string;
  type: string;
  position: CardPosition;
  size: CardSize;
  visible: boolean;
}
/** Columns on the Home grid. Declared before DEFAULT_LAYOUT, which packs at load. */
export const GRID_COLS = 12;

export interface DashboardLayoutData {
  cards: DashboardCardLayout[];
}

// 'hero' (Status/readiness) is gone from this default (2026-09-01) — retired
// in favor of the sidebar's nav status indicator beside Home. Positions are
// recomputed by `reflow` rather than hand-adjusted, same as every other pass
// in this file.
export const DEFAULT_LAYOUT: DashboardLayoutData = {
  cards: reflow([
    { id: 'decisions', type: 'decisions', position: { x: 0, y: 0 }, size: { w: 5, h: 4 }, visible: true },
    { id: 'stats', type: 'stats', position: { x: 0, y: 0 }, size: { w: 5, h: 4 }, visible: true },
    { id: 'calendar', type: 'calendar', position: { x: 0, y: 0 }, size: { w: 7, h: 4 }, visible: true },
    { id: 'council', type: 'council', position: { x: 0, y: 0 }, size: { w: 12, h: 6 }, visible: true },
    { id: 'in_flight', type: 'in_flight', position: { x: 0, y: 0 }, size: { w: 12, h: 3 }, visible: true },
    { id: 'growth_results', type: 'growth_results', position: { x: 0, y: 0 }, size: { w: 12, h: 6 }, visible: true },
    { id: 'recent', type: 'recent', position: { x: 0, y: 0 }, size: { w: 12, h: 4 }, visible: true },
    { id: 'timeline', type: 'timeline', position: { x: 0, y: 0 }, size: { w: 12, h: 6 }, visible: true },
  ]),
};

/** A card's resolved cell on the grid: column/row origin and span. */
export interface PackedCell { id: string; x: number; y: number; w: number; h: number }

/**
 * First-fit ("dense") packing, in array order.
 *
 * Each card goes to the topmost, then leftmost, cell range it fits in — the
 * same rule as CSS `grid-auto-flow: dense`. The previous packer was a shelf:
 * a card that did not fit on the current row opened a new row below the
 * TALLEST card on it, and nothing ever went back. So a 3+4-column row followed
 * by a run of 12-column cards left five empty columns beside it for good, and
 * two side-by-side cards of different heights left a hole under the shorter
 * one (QA walk at 1456px: "Needs you" + "To-dos", then an empty half-row).
 *
 * Array order is still the user's Customize order: it decides who is placed
 * first, so a drag-reorder still moves a card earlier or later on the board.
 */
export function packDense(
  cards: { id: string; size: CardSize }[],
  cols: number = GRID_COLS,
): PackedCell[] {
  const occupied: boolean[][] = [];
  const free = (x: number, y: number, w: number, h: number) => {
    for (let r = y; r < y + h; r++) {
      const row = occupied[r];
      if (!row) continue;
      for (let c = x; c < x + w; c++) if (row[c]) return false;
    }
    return true;
  };
  const mark = (x: number, y: number, w: number, h: number) => {
    for (let r = y; r < y + h; r++) {
      occupied[r] = occupied[r] ?? new Array(cols).fill(false);
      for (let c = x; c < x + w; c++) occupied[r][c] = true;
    }
  };
  return cards.map(card => {
    const w = Math.max(1, Math.min(cols, card.size.w));
    const h = Math.max(1, card.size.h);
    for (let y = 0; ; y++) {
      for (let x = 0; x + w <= cols; x++) {
        if (free(x, y, w, h)) {
          mark(x, y, w, h);
          return { id: card.id, x, y, w, h };
        }
      }
    }
  });
}

/**
 * Stretch packed cards into the holes dense packing cannot fill.
 *
 * Dense packing only fills a hole with a LATER card that fits in it. When the
 * rest of the board is full-width cards (the real Home layout), nothing fits
 * and the hole stays. So, display-only, each card grows right into empty
 * columns beside it, then down into empty rows beneath it (never past the
 * board's last row). The persisted sizes are untouched — this is how the
 * board is drawn, not what the user chose — which is why Customize mode shows
 * the unstretched sizes the resize grip actually edits.
 */
export function fillHoles(cells: PackedCell[], cols: number = GRID_COLS): PackedCell[] {
  const out = cells.map(c => ({ ...c }));
  const rows = out.reduce((m, c) => Math.max(m, c.y + c.h), 0);
  const grid: (number | -1)[][] = Array.from({ length: rows }, () => new Array(cols).fill(-1));
  out.forEach((c, i) => {
    for (let r = c.y; r < c.y + c.h; r++) for (let k = c.x; k < c.x + c.w; k++) grid[r][k] = i;
  });
  const order = out.map((_, i) => i).sort((a, b) => out[a].y - out[b].y || out[a].x - out[b].x);
  // Right first, for every card, so a row's cards share its width before any
  // card claims the rows beneath a neighbour.
  for (const i of order) {
    const c = out[i];
    while (c.x + c.w < cols) {
      let clear = true;
      for (let r = c.y; r < c.y + c.h; r++) if (grid[r][c.x + c.w] !== -1) { clear = false; break; }
      if (!clear) break;
      for (let r = c.y; r < c.y + c.h; r++) grid[r][c.x + c.w] = i;
      c.w += 1;
    }
  }
  for (const i of order) {
    const c = out[i];
    while (c.y + c.h < rows) {
      let clear = true;
      for (let k = c.x; k < c.x + c.w; k++) if (grid[c.y + c.h][k] !== -1) { clear = false; break; }
      if (!clear) break;
      for (let k = c.x; k < c.x + c.w; k++) grid[c.y + c.h][k] = i;
      c.h += 1;
    }
  }
  return out;
}

/**
 * The smallest span a card may be drawn at, for the grid's current width.
 *
 * Twelve fractional columns scale with the window, so a 3-column card that is
 * a comfortable 290px at 1456px is a 200px sliver at ~1100px, where every
 * row of its content ellipses. Below `minCardPx` a card is drawn wider
 * (display-only, like {@link fillHoles}); the dense pack then re-flows around
 * it. Returns 1 when the width is unknown (first paint / jsdom).
 */
export function minSpanForWidth(gridPx: number, gapPx: number, minCardPx = 260): number {
  if (!(gridPx > 0)) return 1;
  const colPx = (gridPx - (GRID_COLS - 1) * gapPx) / GRID_COLS;
  if (colPx <= 0) return GRID_COLS;
  return Math.max(1, Math.min(GRID_COLS, Math.ceil((minCardPx + gapPx) / (colPx + gapPx))));
}

/** Grid cells no card covers, above the board's last row. */
export function countHoles(cells: PackedCell[], cols: number = GRID_COLS): number {
  const rows = cells.reduce((m, c) => Math.max(m, c.y + c.h), 0);
  let covered = 0;
  for (const c of cells) covered += Math.min(c.w, cols) * c.h;
  return rows * cols - covered;
}

/**
 * Justified rows: the hole-free fallback.
 *
 * Dense packing plus {@link fillHoles} can still leave a hole that is boxed in
 * on all four sides (a short card above, a tall one below, neighbours either
 * side), and no rectangle can grow into it. Rows cannot have that: cards are
 * taken in order into a row (a later card that fits the remaining width is
 * pulled forward, as dense packing would), every card in the row takes the
 * row's height, and the row's spare columns are shared out in proportion to
 * each card's own span.
 */
export function justifyRows(
  cards: { id: string; size: CardSize }[],
  cols: number = GRID_COLS,
): PackedCell[] {
  const pending = cards.map((c, i) => ({ i, id: c.id, w: Math.max(1, Math.min(cols, c.size.w)), h: Math.max(1, c.size.h) }));
  const out: PackedCell[] = new Array(cards.length);
  let y = 0;
  while (pending.length) {
    const row = [pending.shift()!];
    let used = row[0].w;
    for (let k = 0; k < pending.length && used < cols; ) {
      if (used + pending[k].w <= cols) {
        used += pending[k].w;
        row.push(pending.splice(k, 1)[0]);
      } else k++;
    }
    const h = Math.max(...row.map(r => r.h));
    const spare = cols - used;
    let x = 0;
    let given = 0;
    row.forEach((r, idx) => {
      const extra = idx === row.length - 1 ? spare - given : Math.floor((spare * r.w) / used);
      given += extra;
      out[r.i] = { id: r.id, x, y, w: r.w + extra, h };
      x += r.w + extra;
    });
    y += h;
  }
  return out;
}

/**
 * Where every visible card is drawn: dense-packed at (at least) `minSpan`
 * columns wide, then — outside Customize — stretched into leftover holes, and
 * if a boxed-in hole survives that, laid out as justified rows instead. The
 * result has no holes at any width.
 */
export function layoutForDisplay(
  cards: { id: string; size: CardSize }[],
  opts: { minSpan?: number; fill?: boolean } = {},
): PackedCell[] {
  const minSpan = Math.min(opts.minSpan ?? 1, GRID_COLS);
  const sized = cards.map(c => ({ id: c.id, size: { w: Math.max(c.size.w, minSpan), h: c.size.h } }));
  const packed = packDense(sized);
  if (opts.fill === false) return packed;
  const filled = fillHoles(packed);
  return countHoles(filled) === 0 ? filled : justifyRows(sized);
}

/**
 * Recompute grid positions from array order with the dense packer, so the
 * persisted positions agree with what {@link layoutForDisplay} draws.
 * Lives here rather than in Dashboard because {@link compactLayoutPass}
 * also needs it — a card that shrinks must not leave a hole behind it.
 */
export function reflow(cards: DashboardCardLayout[]): DashboardCardLayout[] {
  const cells = packDense(cards);
  return cards.map((card, i) => ({ ...card, position: { x: cells[i].x, y: cells[i].y } }));
}

/**
 * Compact target sizes, in grid units (12 columns, 60px rows).
 *
 * These are CEILINGS applied once, not fixed sizes: a card larger than its
 * entry is shrunk to it, a card already smaller is left alone. The numbers
 * come from what each card actually needs to show its content without a void —
 * ambient readouts are two rows, stat blocks three, lists five.
 */
const COMPACT_SIZES: Record<string, CardSize> = {
  weather: { w: 3, h: 2 },
  system_stats: { w: 3, h: 2 },
  hero: { w: 6, h: 3 },
  decisions: { w: 3, h: 3 },
  stats: { w: 4, h: 3 },
  in_flight: { w: 5, h: 3 },
  todos: { w: 4, h: 5 },
  calendar: { w: 4, h: 5 },
  recent: { w: 8, h: 5 },
  timeline: { w: 12, h: 5 },
  council: { w: 12, h: 5 },
};

/**
 * First-party card types the registry no longer offers. A layout persisted
 * before a retirement still names them, and without this the grid asks
 * `Component: registry[card.type]?.component`, gets nothing, and shows a
 * MissingCard "Card no longer available" ghost tile forever (registry.ts has
 * no entry to add back). Filtering them out of the loaded layout is the
 * graceful-degrade path instead: the retired card simply is not there, same
 * as if the user had removed it in Customize.
 *
 * No one-time-pass marker (unlike the migrations below): this is safe and
 * cheap to re-check on every load — once a layout has none of these types it
 * is a no-op forever, and a future retirement only has to add a type here.
 */
const RETIRED_CARD_TYPES = new Set<string>(['hero']);

export function stripRetiredCards(
  layout: DashboardLayoutData,
): { layout: DashboardLayoutData; changed: boolean } {
  if (!layout.cards.some(c => RETIRED_CARD_TYPES.has(c.type))) {
    return { layout, changed: false };
  }
  const cards = layout.cards.filter(c => !RETIRED_CARD_TYPES.has(c.type));
  return { layout: { cards: reflow(cards) }, changed: true };
}

/**
 * Marker for the one-time compaction. Without it the pass would re-apply on
 * every load and silently undo any card the user deliberately enlarged
 * afterwards — a "helpful" migration that never stops helping is a bug.
 * Client-side because it is a UI preference, not shared state.
 */
const COMPACT_PASS_KEY = 'permagent.dashboard.compactPass.v1';
const GROWTH_CARD_PASS_KEY = 'permagent.dashboard.growthResultsCard.v1';
const GROWTH_CARD_TALL_PASS_KEY = 'permagent.dashboard.growthResultsCard.tall.v1';
const CALENDAR_PASS_KEY = 'permagent.dashboard.calendarPass.v1';
const COUNCIL_CARD_PASS_KEY = 'permagent.dashboard.councilCard.v1';

export function hasRunGrowthCardPass(): boolean {
  try { return localStorage.getItem(GROWTH_CARD_PASS_KEY) === '1'; } catch { return false; }
}

export function markGrowthCardPassDone(): void {
  try { localStorage.setItem(GROWTH_CARD_PASS_KEY, '1'); } catch { /* private mode — retry next load */ }
}

/**
 * Existing dashboards never got a Growth card because DEFAULT_LAYOUT only
 * applies to first paint. Add it once; if the user later removes it, the
 * marker keeps this from putting it back.
 */
export function ensureGrowthResultsCard(
  layout: DashboardLayoutData,
): { layout: DashboardLayoutData; changed: boolean } {
  if (layout.cards.some((c) => c.type === 'growth_results')) {
    return { layout, changed: false };
  }
  const cards = [
    ...layout.cards,
    {
      id: 'growth_results',
      type: 'growth_results',
      position: { x: 0, y: 0 },
      size: { w: 12, h: 6 },
      visible: true,
    },
  ];
  return { layout: { cards: reflow(cards) }, changed: true };
}

export function hasRunGrowthCardTallPass(): boolean {
  try { return localStorage.getItem(GROWTH_CARD_TALL_PASS_KEY) === '1'; } catch { return false; }
}

export function markGrowthCardTallPassDone(): void {
  try { localStorage.setItem(GROWTH_CARD_TALL_PASS_KEY, '1'); } catch { /* private mode — retry next load */ }
}

/**
 * The first Growth card was 12×4, which is too short for a fleet trend plus
 * per-project sparklines. Grow it once if it is still that default; a card
 * the user already resized is left alone.
 */
export function ensureGrowthCardTallEnough(
  layout: DashboardLayoutData,
): { layout: DashboardLayoutData; changed: boolean } {
  let changed = false;
  const cards = layout.cards.map((card) => {
    if (card.type !== 'growth_results' || card.size.w !== 12 || card.size.h !== 4) {
      return card;
    }
    changed = true;
    return { ...card, size: { w: 12, h: 6 } };
  });
  if (!changed) return { layout, changed: false };
  return { layout: { cards: reflow(cards) }, changed: true };
}

export function hasRunCompactPass(): boolean {
  try { return localStorage.getItem(COMPACT_PASS_KEY) === '1'; } catch { return false; }
}

export function markCompactPassDone(): void {
  try { localStorage.setItem(COMPACT_PASS_KEY, '1'); } catch { /* private mode — retry next load */ }
}

export function hasRunCalendarPass(): boolean {
  try { return localStorage.getItem(CALENDAR_PASS_KEY) === '1'; } catch { return false; }
}

export function markCalendarPassDone(): void {
  try { localStorage.setItem(CALENDAR_PASS_KEY, '1'); } catch { /* private mode — retry next load */ }
}

/** Insert the Calendar card once if the persisted layout never had one. */
export function ensureCalendarCard(
  layout: DashboardLayoutData,
): { layout: DashboardLayoutData; changed: boolean } {
  if (layout.cards.some(c => c.type === 'calendar')) return { layout, changed: false };
  return {
    layout: {
      cards: reflow([
        ...layout.cards,
        { id: 'calendar', type: 'calendar', position: { x: 0, y: 0 }, size: { w: 5, h: 4 }, visible: true },
      ]),
    },
    changed: true,
  };
}

export function hasRunCouncilCardPass(): boolean {
  try { return localStorage.getItem(COUNCIL_CARD_PASS_KEY) === '1'; } catch { return false; }
}

export function markCouncilCardPassDone(): void {
  try { localStorage.setItem(COUNCIL_CARD_PASS_KEY, '1'); } catch { /* private mode — retry next load */ }
}

/** Insert the Council card once if the persisted layout never had one. */
export function ensureCouncilCard(
  layout: DashboardLayoutData,
): { layout: DashboardLayoutData; changed: boolean } {
  if (layout.cards.some(c => c.type === 'council')) return { layout, changed: false };
  return {
    layout: {
      cards: reflow([
        ...layout.cards,
        { id: 'council', type: 'council', position: { x: 0, y: 0 }, size: { w: 12, h: 6 }, visible: true },
      ]),
    },
    changed: true,
  };
}

/**
 * Shrink oversized cards to their compact ceiling, once.
 *
 * The earlier version only touched cards sitting at EXACTLY the old 5x4
 * default. That was too narrow to be useful: real dashboards had been
 * auto-arranged to 4x3 and 4x6, matched nothing, and nothing shrank — the
 * change shipped and was invisible. Clamping "anything larger than the
 * ceiling" is what the user actually asked for, and the run-once marker is
 * what keeps it from becoming an override.
 */
export function compactLayoutPass(
  layout: DashboardLayoutData,
): { layout: DashboardLayoutData; changed: boolean } {
  let changed = false;
  const cards = layout.cards.map(card => {
    const target = COMPACT_SIZES[card.type];
    if (!target) return card;
    const w = Math.min(card.size.w, target.w);
    const h = Math.min(card.size.h, target.h);
    if (w === card.size.w && h === card.size.h) return card;
    changed = true;
    return { ...card, size: { w, h } };
  });
  if (!changed) return { layout, changed: false };
  return { layout: { ...layout, cards: reflow(cards) }, changed: true };
}

export function useLayout() {
  const [layout, setLayout] = useState<DashboardLayoutData>(DEFAULT_LAYOUT);

  useEffect(() => {
    let cancelled = false;
    apiFetch<DashboardLayoutData>('/api/dashboard/layout')
      .then(fetched => {
        if (cancelled) return;
        // Strip any retired first-party card (e.g. 'hero') before anything
        // else touches the layout — every pass below should only ever see
        // card types the registry can still render.
        const stripped = stripRetiredCards(fetched);
        // Run the compaction at most once per machine; afterwards the user's
        // own sizing is authoritative.
        const compacted = hasRunCompactPass()
          ? { layout: stripped.layout, changed: false }
          : compactLayoutPass(stripped.layout);
        let normalized = compacted.layout;
        let changed = stripped.changed || compacted.changed;
        if (!hasRunGrowthCardPass()) {
          const inserted = ensureGrowthResultsCard(normalized);
          normalized = inserted.layout;
          changed = changed || inserted.changed;
        }
        if (!hasRunGrowthCardTallPass()) {
          const taller = ensureGrowthCardTallEnough(normalized);
          normalized = taller.layout;
          changed = changed || taller.changed;
        }
        if (!hasRunCalendarPass()) {
          const calendar = ensureCalendarCard(normalized);
          normalized = calendar.layout;
          changed = changed || calendar.changed;
        }
        if (!hasRunCouncilCardPass()) {
          const council = ensureCouncilCard(normalized);
          normalized = council.layout;
          changed = changed || council.changed;
        }
        setLayout(normalized);
        markCompactPassDone();
        markGrowthCardPassDone();
        markGrowthCardTallPassDone();
        markCalendarPassDone();
        markCouncilCardPassDone();
        if (changed) {
          apiFetch<DashboardLayoutData>('/api/dashboard/layout', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(normalized),
          }).catch(() => { /* cosmetic; retried next load */ });
        }
      })
      .catch(() => { /* use default */ });
    return () => { cancelled = true; };
  }, []);

  const persistLayout = useCallback(async (newLayout: DashboardLayoutData) => {
    setLayout(newLayout);
    await apiFetch<DashboardLayoutData>('/api/dashboard/layout', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(newLayout),
    });
  }, []);

  return { layout, setLayout, persistLayout };
}
