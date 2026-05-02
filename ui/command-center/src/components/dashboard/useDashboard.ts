import { useState, useEffect, useRef } from 'react';

export interface DashboardAgent { name: string; state: string; active_count: number; summary: string }
export interface DashboardStats { sessions_today: number; sessions_total: number; memory_count: number; memory_delta_today: number }
export interface InFlightSession { id: string; title: string; started_at: string; state: string; progress: number }
export interface RecentSession { id: string; title: string; state: string; ended_at: string }
export interface DashboardData { agent: DashboardAgent; stats: DashboardStats; in_flight: InFlightSession[]; recent: RecentSession[] }

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const API = (import.meta.env.VITE_DAEMON_URL as string | undefined) ||
  (import.meta.env.VITE_API_BASE_URL as string | undefined) ||
  (isTauri ? 'http://127.0.0.1:3001' : '');

export function useDashboard() {
  const [data, setData] = useState<DashboardData | null>(null);
  const [loading, setLoading] = useState(true);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();

  const fetchDashboard = async () => {
    try {
      const res = await fetch(`${API}/api/dashboard`);
      if (res.ok) setData(await res.json());
    } catch { /* ignore */ }
    setLoading(false);
  };

  useEffect(() => {
    fetchDashboard();
    intervalRef.current = setInterval(fetchDashboard, 15_000);
    return () => clearInterval(intervalRef.current);
  }, []);

  return { data, loading, refresh: fetchDashboard };
}
