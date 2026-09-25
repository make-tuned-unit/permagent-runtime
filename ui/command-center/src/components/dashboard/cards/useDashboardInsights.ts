import { useCallback, useEffect, useRef, useState } from 'react';
import { apiFetch } from '../../../lib/api';

/**
 * `GET /api/dashboard/insights` — the one aggregation endpoint backing
 * `today_focus`, `overnight_digest`, `fleet_kpis`, `stalled_pattern` and
 * `project_health_grid` (dagK home-templates lane, converging what the
 * research doc specced as 5 separate endpoints into 1, per R7: these cards
 * all read the same underlying goals/decisions/projects, so one shared fetch
 * here replaces what would otherwise be 5 client fan-outs on every Home
 * load). Every field is a deterministic count or cluster from real rows — see
 * `permagent::dashboard_insights` for the honest-data rules (P12).
 */
export interface TodayFocus {
  kind: 'decision' | 'goal' | null;
  refId: string | null;
  projectId: string | null;
  title: string | null;
  reason: string | null;
}
export interface OvernightDigest {
  newDecisions: number;
  stalledSince: number;
  quiet: boolean;
}
export interface FleetKpis {
  projectCount: number;
  activeGoalCount: number;
  stalledGoalCount: number;
  pendingDecisionCount: number;
  sessionsToday: number;
}
export interface StalledGoalRef {
  goalId: string;
  projectId: string;
}
export interface StalledCluster {
  keyword: string;
  goals: StalledGoalRef[];
}
export interface ProjectHealthRow {
  projectId: string;
  projectName: string;
  activeGoals: number;
  heldGoals: number;
  status: 'active' | 'stalled' | 'dormant';
  lastTouched: string | null;
}
export interface DashboardInsights {
  todayFocus: TodayFocus;
  overnightDigest: OvernightDigest;
  fleetKpis: FleetKpis;
  stalledPatterns: StalledCluster[];
  projectHealthGrid: ProjectHealthRow[];
}

const POLL_MS = 30_000;

export function useDashboardInsights() {
  const [data, setData] = useState<DashboardInsights | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const generation = useRef(0);

  const fetchInsights = useCallback(async () => {
    const gen = ++generation.current;
    try {
      const result = await apiFetch<DashboardInsights>('/api/dashboard/insights');
      if (gen !== generation.current) return;
      setData(result);
      setError(false);
    } catch {
      if (gen !== generation.current) return;
      setError(true);
    }
    if (gen === generation.current) setLoading(false);
  }, []);

  useEffect(() => {
    fetchInsights();
    const id = setInterval(fetchInsights, POLL_MS);
    return () => { clearInterval(id); ++generation.current; };
  }, [fetchInsights]);

  return { data, loading, error: error && data === null, refresh: fetchInsights };
}
