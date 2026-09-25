/**
 * Shared shape for the growth-results dashboard (this project + fleet).
 *
 * Mirrors `GrowthResults` in routes/growth_actions.rs. Derived from
 * `growth_action_outcomes` — never authored by a model — so a card that says
 * "helped" is a measured window, not a suggestion.
 */

/**
 * The daemon used to store action titles through `title.chars().take(120)`,
 * which cut real titles mid-word with no ellipsis ("…single-event funnel and",
 * "…(ChatGPT, Perplexit"). New titles are clipped at a word boundary server
 * side (growth_actions.rs `clip_at_word`); rows written before that fix are
 * exactly 120 characters, so they are trimmed back to the last whole word and
 * marked "…" here. Anything shorter or already punctuated is shown as is.
 */
export const LEGACY_TITLE_CAP = 120;

export function displayActionTitle(title: string): string {
  const chars = Array.from(title);
  if (chars.length !== LEGACY_TITLE_CAP || /[.!?…)\]"']$/.test(title)) return title;
  const cut = title.replace(/\s+\S*$/, '').replace(/[\s,;:(—-]+$/, '');
  return `${cut || title}…`;
}

/** Title style shared by the Results and Actions cards: wraps, and clamps at
 *  three lines with an ellipsis instead of running the card tall. */
export const ACTION_TITLE_CLAMP = {
  display: '-webkit-box',
  WebkitLineClamp: 3,
  WebkitBoxOrient: 'vertical',
  overflow: 'hidden',
  overflowWrap: 'anywhere',
} as const;

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

/** The `/api/growth-actions/measure` reply — the same nightly sweep, run on
 *  demand instead of waiting for the cron window. */
export interface MeasureNowResult {
  actionsConsidered: number;
  windowsJudged: number;
  actionsCompleted: number;
  errors: string[];
}

export function formatDeltaPct(delta: number | null | undefined): string | null {
  if (delta == null || Number.isNaN(delta)) return null;
  const pct = delta * 100;
  const sign = pct > 0 ? '+' : '';
  return `${sign}${pct.toFixed(0)}%`;
}
