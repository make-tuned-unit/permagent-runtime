/** Layout choices for Home. The daily default is owned by this frontend;
 * other templates can be supplied by the daemon, with bundled fallbacks. */
import { useEffect, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { DEFAULT_LAYOUT, reflow, type DashboardCardLayout, type DashboardLayoutData } from './useLayout';

export interface TemplateCard {
  type: string;
  id: string;
  position: { x: number; y: number };
  size: { w: number; h: number };
  comingSoon?: boolean;
}

export interface DashboardTemplate {
  id: string;
  name: string;
  question: string;
  cards: TemplateCard[];
}

export const DASHBOARD_TEMPLATES: DashboardTemplate[] = [
  {
    id: 'morning_briefing',
    name: 'Daily overview',
    question: 'What needs me, and what is happening today?',
    cards: DEFAULT_LAYOUT.cards.map(({ visible: _visible, ...card }) => card),
  },
  {
    id: 'operator_command_center',
    name: 'Operator Command Center',
    question: "What's the state of all my projects, and where's the risk?",
    cards: [
      { type: 'fleet_kpis', id: 'fleet_kpis', position: { x: 0, y: 0 }, size: { w: 12, h: 2 } },
      { type: 'stalled_pattern', id: 'stalled_pattern', position: { x: 0, y: 2 }, size: { w: 12, h: 3 } },
      { type: 'project_health_grid', id: 'project_health_grid', position: { x: 0, y: 5 }, size: { w: 12, h: 5 } },
      { type: 'in_flight', id: 'in_flight', position: { x: 0, y: 10 }, size: { w: 8, h: 3 } },
      { type: 'job_health', id: 'job_health', position: { x: 8, y: 10 }, size: { w: 4, h: 3 } },
      { type: 'growth_results', id: 'growth_results', position: { x: 0, y: 13 }, size: { w: 12, h: 6 } },
    ],
  },
  {
    id: 'weekly_story',
    name: 'Weekly Story',
    question: 'What actually happened this week, and how does it compare?',
    cards: [
      { type: 'week_headline', id: 'week_headline', position: { x: 0, y: 0 }, size: { w: 12, h: 2 } },
      { type: 'timeline', id: 'timeline', position: { x: 0, y: 2 }, size: { w: 12, h: 6 } },
      { type: 'week_small_multiples', id: 'week_small_multiples', position: { x: 0, y: 8 }, size: { w: 12, h: 3 } },
      { type: 'council', id: 'council', position: { x: 0, y: 11 }, size: { w: 12, h: 5 } },
      { type: 'memory_growth', id: 'memory_growth', position: { x: 0, y: 16 }, size: { w: 6, h: 3 } },
      { type: 'on_this_week', id: 'on_this_week', position: { x: 6, y: 16 }, size: { w: 6, h: 3 } },
    ],
  },
  {
    id: 'relationships_commitments',
    name: 'Relationships & Commitments',
    question: 'Who am I neglecting, and what did I promise?',
    cards: [
      { type: 'people_attention', id: 'people_attention', position: { x: 0, y: 0 }, size: { w: 6, h: 5 } },
      { type: 'open_commitments', id: 'open_commitments', position: { x: 6, y: 0 }, size: { w: 6, h: 5 } },
      { type: 'recent_meetings', id: 'recent_meetings', position: { x: 0, y: 5 }, size: { w: 6, h: 4 } },
      { type: 'calendar', id: 'calendar', position: { x: 6, y: 5 }, size: { w: 6, h: 8 } },
    ],
  },
  {
    id: 'growth_and_money',
    name: 'Growth & Money',
    question: 'Is the work paying off?',
    cards: [
      { type: 'growth_results', id: 'growth_results', position: { x: 0, y: 0 }, size: { w: 12, h: 6 } },
      { type: 'finance_summary', id: 'finance_summary', position: { x: 0, y: 6 }, size: { w: 8, h: 5 } },
      { type: 'finance_watchlist', id: 'finance_watchlist', position: { x: 8, y: 6 }, size: { w: 4, h: 5 } },
      { type: 'cost_vs_outcome', id: 'cost_vs_outcome', position: { x: 0, y: 11 }, size: { w: 12, h: 3 } },
    ],
  },
];

/** The cards a template would actually render — `comingSoon` ones left out,
 *  same rule the agent's `preview_dashboard_layout`/`apply_dashboard_layout`
 *  tools apply server-side, densely re-packed so no gap is left where an
 *  unbuilt card would have been. */
export function realizedLayout(template: DashboardTemplate): DashboardLayoutData {
  const real = template.cards.filter(c => !c.comingSoon);
  const cards: DashboardCardLayout[] = real.map(c => ({
    id: c.id,
    type: c.type,
    position: c.position,
    size: c.size,
    visible: true,
  }));
  return { cards: reflow(cards) };
}

/** Names of cards a template lists that Phase 2 hasn't built yet. */
export function comingSoonCards(template: DashboardTemplate): string[] {
  return template.cards.filter(c => c.comingSoon).map(c => c.type);
}

/**
 * Human names for the Phase-2 card types that don't have a registry entry
 * yet (a `comingSoon` card is, by definition, not registered — so
 * `useCardRegistry()` has nothing to look its name up in). Kept here, next to
 * the templates that name them, so a Phase-2 card ships its display name in
 * the same commit that removes it from this list (coordinator, Phase-1
 * presentation pass: "'Coming soon' must use human card names, not
 * identifiers").
 */
const COMING_SOON_NAMES: Record<string, string> = {
  week_headline: 'Week Headline',
  week_small_multiples: 'Week Small Multiples',
  memory_growth: 'Memory Growth',
  on_this_week: 'On This Week',
  people_attention: 'Needs Attention — People',
  open_commitments: 'Open Commitments',
  recent_meetings: 'Recent Meetings',
  finance_summary: 'Finance Board',
  finance_watchlist: 'Watchlist',
  cost_vs_outcome: 'Cost vs. Outcome',
};

/** Human name for any card type, built-in or not-yet-built. */
export function cardDisplayName(type: string): string {
  return COMING_SOON_NAMES[type] ?? type;
}

/** Whether `template`'s realized card-type set matches the layout on screen
 *  — the gallery's "this is what you have" marker. Position-agnostic (a
 *  reflow doesn't change identity) and order-agnostic, by design: what makes
 *  a layout "the current one" is which cards are on it, not their exact
 *  pixel order after a resize. */
export function isCurrentTemplate(template: DashboardTemplate, currentLayout: DashboardLayoutData): boolean {
  const templateTypes = [...new Set(realizedLayout(template).cards.map(c => c.type))].sort();
  const currentTypes = [...new Set(currentLayout.cards.map(c => c.type))].sort();
  return templateTypes.length === currentTypes.length && templateTypes.every((t, i) => t === currentTypes[i]);
}

/** Keep the daily default current even when connected to an older daemon. */
export function mergeDashboardTemplates(remote: DashboardTemplate[]): DashboardTemplate[] {
  const daily = DASHBOARD_TEMPLATES[0];
  return [daily, ...remote.filter(template => template.id !== daily.id)];
}

/**
 * Live templates from the daemon, retaining the frontend daily default so
 * the gallery still renders with the daemon unreachable (offline, or the
 * harness). Fetched once — templates don't change at runtime.
 */
export function useDashboardTemplates(): DashboardTemplate[] {
  const [templates, setTemplates] = useState<DashboardTemplate[]>(DASHBOARD_TEMPLATES);
  useEffect(() => {
    let cancelled = false;
    apiFetch<DashboardTemplate[]>('/api/dashboard/templates')
      .then(result => { if (!cancelled && Array.isArray(result) && result.length > 0) setTemplates(mergeDashboardTemplates(result)); })
      .catch(() => { /* keep the static mirror */ });
    return () => { cancelled = true; };
  }, []);
  return templates;
}
