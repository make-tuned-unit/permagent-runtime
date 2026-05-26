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
    { id: 'stats', type: 'stats', position: { x: 7, y: 0 }, size: { w: 5, h: 4 }, visible: true },
    { id: 'in_flight', type: 'in_flight', position: { x: 0, y: 4 }, size: { w: 12, h: 3 }, visible: true },
    { id: 'recent', type: 'recent', position: { x: 0, y: 7 }, size: { w: 12, h: 4 }, visible: true },
  ],
};

export function useLayout() {
  const [layout, setLayout] = useState<DashboardLayoutData>(DEFAULT_LAYOUT);

  useEffect(() => {
    apiFetch<DashboardLayoutData>('/api/dashboard/layout')
      .then(setLayout)
      .catch(() => { /* use default */ });
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
