import { useState, useEffect, useRef, useCallback } from 'react';
import { apiFetch } from '../../lib/api';

export interface DashboardAgent { name: string; state: string; active_count: number; summary: string }
export interface DashboardStats { sessions_today: number; sessions_total: number; memory_count: number; memory_delta_today: number }
export interface InFlightSession { id: string; title: string; started_at: string; state: string; progress: number }
export interface RecentSession {
  id: string;
  /** dashboard.rs's display title (the name cut at 40, or `Session <id>`). */
  title: string;
  state: string;
  ended_at: string;
  // The same naming/classifying fields /api/sessions carries (SessionSummary),
  // so Home labels and filters a session exactly as History does. Optional:
  // an older daemon sends only the four above.
  name?: string | null;
  user_set_name?: boolean;
  session_type?: string;
  schedule_id?: string | null;
  recipe_title?: string | null;
  first_user_text?: string | null;
  goal_id?: string | null;
  goal_title?: string | null;
  project_name?: string | null;
}
export interface DashboardData { agent: DashboardAgent; stats: DashboardStats; in_flight: InFlightSession[]; recent: RecentSession[] }

/** A dashboard read that has not answered by now is treated as failed. Without
 * a limit, a daemon whose /api/dashboard never answers left Home on the loading
 * logo forever, with no error and no Try again. */
export const DASHBOARD_TIMEOUT_MS = 20_000;

export function useDashboard() {
  const [data, setData] = useState<DashboardData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  /** When the figures currently on screen were last confirmed true. */
  const [lastOkAt, setLastOkAt] = useState<number | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();
  const requestGeneration = useRef(0);
  /** A read is outstanding. The 15 s poll must not stack a new read on a slow
   *  one — each new read supersedes the older, so a hung daemon's timeouts
   *  would always be discarded and the failure never surfaced. */
  const inFlight = useRef(false);

  const fetchDashboard = useCallback(async () => {
    const generation = ++requestGeneration.current;
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), DASHBOARD_TIMEOUT_MS);
    inFlight.current = true;
    try {
      const result = await apiFetch<DashboardData>('/api/dashboard', { signal: controller.signal });
      if (generation !== requestGeneration.current) return;
      setData(result);
      setError(false);
      setLastOkAt(Date.now());
    } catch {
      if (generation !== requestGeneration.current) return;
      // Surface the failure instead of spinning forever — a failed poll while
      // data is already on screen keeps the stale dashboard, not an alarm.
      setError(true);
    } finally {
      clearTimeout(timeout);
      if (generation === requestGeneration.current) inFlight.current = false;
    }
    if (generation !== requestGeneration.current) return;
    setLoading(false);
  }, []);

  useEffect(() => {
    fetchDashboard();
    intervalRef.current = setInterval(() => { if (!inFlight.current) void fetchDashboard(); }, 15_000);
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
