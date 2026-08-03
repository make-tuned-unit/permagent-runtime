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
export interface DashboardLayoutData {
  cards: DashboardCardLayout[];
}

export const DEFAULT_LAYOUT: DashboardLayoutData = {
  cards: [
    { id: 'hero', type: 'hero', position: { x: 0, y: 0 }, size: { w: 7, h: 4 }, visible: true },
    { id: 'decisions', type: 'decisions', position: { x: 7, y: 0 }, size: { w: 5, h: 4 }, visible: true },
    { id: 'stats', type: 'stats', position: { x: 0, y: 4 }, size: { w: 5, h: 4 }, visible: true },
    { id: 'in_flight', type: 'in_flight', position: { x: 0, y: 8 }, size: { w: 12, h: 3 }, visible: true },
    { id: 'recent', type: 'recent', position: { x: 0, y: 11 }, size: { w: 12, h: 4 }, visible: true },
    { id: 'timeline', type: 'timeline', position: { x: 0, y: 15 }, size: { w: 12, h: 6 }, visible: true },
  ],
};

/**
 * Recompute grid positions from array order, packing cards into 12-column rows.
 * Lives here rather than in Dashboard because {@link normalizeCompactCards}
 * also needs it — a card that shrinks must not leave a hole behind it.
 */
export function reflow(cards: DashboardCardLayout[]): DashboardCardLayout[] {
  let x = 0;
  let y = 0;
  let rowHeight = 0;
  return cards.map(card => {
    const w = card.size.w;
    const h = card.size.h;
    if (x + w > 12) {
      x = 0;
      y += rowHeight;
      rowHeight = 0;
    }
    const placed = { ...card, position: { x, y } };
    x += w;
    rowHeight = Math.max(rowHeight, h);
    return placed;
  });
}

/**
 * Card types that became small compact tiles, and the exact size they used to
 * occupy. Weather and machine load are ambient readouts — glanced at, never
 * acted on — and at 5x4 each was taking the same room as the decision queue.
 */
const COMPACT_MIGRATIONS: Record<string, { from: CardSize; to: CardSize }> = {
  weather: { from: { w: 5, h: 4 }, to: { w: 3, h: 2 } },
  system_stats: { from: { w: 5, h: 4 }, to: { w: 3, h: 2 } },
};

/**
 * One-time shrink of ambient cards already placed on a saved dashboard.
 *
 * A manifest's `defaultSize` only applies when a card is ADDED, so without this
 * the change would reach new dashboards and never the ones that already exist.
 *
 * It only touches a card still sitting at EXACTLY the old default size. Anyone
 * who deliberately resized their weather card has said what they want, and a
 * "migration" that overrode that would be a bug wearing a helpful hat.
 */
export function normalizeCompactCards(
  layout: DashboardLayoutData,
): { layout: DashboardLayoutData; changed: boolean } {
  let changed = false;
  const cards = layout.cards.map(card => {
    const rule = COMPACT_MIGRATIONS[card.type];
    if (!rule) return card;
    if (card.size.w !== rule.from.w || card.size.h !== rule.from.h) return card;
    changed = true;
    return { ...card, size: { ...rule.to } };
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
        const { layout: normalized, changed } = normalizeCompactCards(fetched);
        setLayout(normalized);
        // Persist the shrink once so it doesn't recompute on every load — and
        // so a later deliberate resize isn't undone next time.
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
