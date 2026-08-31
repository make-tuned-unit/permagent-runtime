import { useState, useEffect, useRef, useCallback } from 'react';
import { apiFetch } from '../../lib/api';

export interface DashboardAgent { name: string; state: string; active_count: number; summary: string }
export interface DashboardStats { sessions_today: number; sessions_total: number; memory_count: number; memory_delta_today: number }
export interface InFlightSession { id: string; title: string; started_at: string; state: string; progress: number }
export interface RecentSession { id: string; title: string; state: string; ended_at: string }
export interface DashboardData { agent: DashboardAgent; stats: DashboardStats; in_flight: InFlightSession[]; recent: RecentSession[] }

/**
 * What the header says about the figures below it.
 *
 * `null` while the poll is healthy — a live number needs no caption, and a
 * timestamp that is always there stops being read. It appears only once the
 * numbers have stopped being refreshed, which is exactly when a user would
 * otherwise have no way to tell: Home keeps the last good payload on a failed
 * poll, so without this the landing page shows a frozen session count in the
 * same type as a live one, indefinitely.
 */
export function dashboardFreshness(
  lastOkAt: number | null,
  failing: boolean,
  now = Date.now(),
): { label: string; stale: boolean } | null {
  if (!failing) return null;
  if (lastOkAt == null) return { label: "Can't reach the daemon · reconnecting", stale: true };
  return { label: `Updated ${sinceLabel(now - lastOkAt)} · reconnecting`, stale: true };
}

function sinceLabel(ms: number): string {
  const mins = Math.floor(ms / 60_000);
  if (mins < 1) return 'moments ago';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export function useDashboard() {
  const [data, setData] = useState<DashboardData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  /** When the figures currently on screen were last confirmed true. */
  const [lastOkAt, setLastOkAt] = useState<number | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();
  const requestGeneration = useRef(0);

  const fetchDashboard = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const result = await apiFetch<DashboardData>('/api/dashboard');
      if (generation !== requestGeneration.current) return;
      setData(result);
      setError(false);
      setLastOkAt(Date.now());
    } catch {
      if (generation !== requestGeneration.current) return;
      // Surface the failure instead of spinning forever — a failed poll while
      // data is already on screen keeps the stale dashboard, not an alarm.
      setError(true);
    }
    if (generation !== requestGeneration.current) return;
    setLoading(false);
  }, []);

  useEffect(() => {
    fetchDashboard();
    intervalRef.current = setInterval(fetchDashboard, 15_000);
    return () => {
      clearInterval(intervalRef.current);
      ++requestGeneration.current;
    };
  }, [fetchDashboard]);

  const retry = () => { setLoading(true); fetchDashboard(); };

  return {
    data, loading, error, lastOkAt, retry, refresh: fetchDashboard,
    /** Polling is failing while figures are still on screen — the case where
     *  the page looks fine and is not. */
    failing: error && data !== null,
  };
}
