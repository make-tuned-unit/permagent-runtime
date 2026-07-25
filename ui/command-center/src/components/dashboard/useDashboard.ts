import { useState, useEffect, useRef, useCallback } from 'react';
import { apiFetch } from '../../lib/api';

export interface DashboardAgent { name: string; state: string; active_count: number; summary: string }
export interface DashboardStats { sessions_today: number; sessions_total: number; memory_count: number; memory_delta_today: number }
export interface InFlightSession { id: string; title: string; started_at: string; state: string; progress: number }
export interface RecentSession { id: string; title: string; state: string; ended_at: string }
export interface DashboardData { agent: DashboardAgent; stats: DashboardStats; in_flight: InFlightSession[]; recent: RecentSession[] }

export function useDashboard() {
  const [data, setData] = useState<DashboardData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();
  const requestGeneration = useRef(0);

  const fetchDashboard = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const result = await apiFetch<DashboardData>('/api/dashboard');
      if (generation !== requestGeneration.current) return;
      setData(result);
      setError(false);
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

  return { data, loading, error, retry, refresh: fetchDashboard };
}
