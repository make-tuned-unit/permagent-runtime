/**
 * Shared shape for the growth-results dashboard (this project + fleet).
 *
 * Mirrors `GrowthResults` in routes/growth_actions.rs. Derived from
 * `growth_action_outcomes` — never authored by a model — so a card that says
 * "helped" is a measured window, not a suggestion.
 */

export interface GrowthResultRow {
  actionId: string;
  projectId: string;
  projectName: string;
  title: string;
  category: string;
  status: string;
  verdict: string | null;
  deltaPct: number | null;
  windowDays: number | null;
  judgedAt: string | null;
  targetMetric: string | null;
  targetDir: string | null;
}

export interface GrowthCategorySummary {
  category: string;
  projects: number;
  helped: number;
  hindered: number;
  noEffect: number;
  medianDeltaPct: number | null;
}

export interface GrowthTrendPoint {
  week: string;
  helped: number;
  hindered: number;
  noEffect: number;
  net: number;
  cumulativeNet: number;
}

export interface GrowthProjectTrend {
  projectId: string;
  projectName: string;
  helped: number;
  hindered: number;
  noEffect: number;
  points: GrowthTrendPoint[];
}

export interface GrowthProjectResults {
  projectId: string;
  name: string;
  segmentLabel: string;
  implemented: number;
  measuring: number;
  judged: number;
  helped: number;
  hindered: number;
  noEffect: number;
  inconclusive: number;
  actions: GrowthResultRow[];
}

export interface GrowthFleetResults {
  projects: number;
  helped: number;
  hindered: number;
  noEffect: number;
  inconclusive: number;
  categories: GrowthCategorySummary[];
  recent: GrowthResultRow[];
  trend: GrowthTrendPoint[];
  byProject: GrowthProjectTrend[];
}

export interface GrowthResultsData {
  project: GrowthProjectResults | null;
  fleet: GrowthFleetResults;
}

export function formatDeltaPct(delta: number | null | undefined): string | null {
  if (delta == null || Number.isNaN(delta)) return null;
  const pct = delta * 100;
  const sign = pct > 0 ? '+' : '';
  return `${sign}${pct.toFixed(0)}%`;
}
