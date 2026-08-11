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
};

/**
 * Marker for the one-time compaction. Without it the pass would re-apply on
 * every load and silently undo any card the user deliberately enlarged
 * afterwards — a "helpful" migration that never stops helping is a bug.
 * Client-side because it is a UI preference, not shared state.
 */
const COMPACT_PASS_KEY = 'permagent.dashboard.compactPass.v1';

export function hasRunCompactPass(): boolean {
  try { return localStorage.getItem(COMPACT_PASS_KEY) === '1'; } catch { return false; }
}

export function markCompactPassDone(): void {
  try { localStorage.setItem(COMPACT_PASS_KEY, '1'); } catch { /* private mode — retry next load */ }
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
        // Run the compaction at most once per machine; afterwards the user's
        // own sizing is authoritative.
        const { layout: normalized, changed } = hasRunCompactPass()
          ? { layout: fetched, changed: false }
          : compactLayoutPass(fetched);
        setLayout(normalized);
        markCompactPassDone();
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
