/**
 * The five Home dashboard layout templates (HOME_TEMPLATES_RESEARCH.md §2) —
 * the frontend's hand-synced mirror of the daemon's
 * `crates/goose-server/src/routes/dashboard.rs::dashboard_templates()`, the
 * same two-copies-in-sync pattern this file's own `DEFAULT_LAYOUT` already
 * keeps with `DashboardLayout::default_layout` there. `useDashboardTemplates`
 * below prefers the LIVE daemon list (so a future template addition needs no
 * frontend rebuild) and falls back to this static list when the daemon can't
 * be reached — the Layouts gallery must still work offline/in the harness.
 *
 * A card `comingSoon: true` names a real card from the research doc that
 * Phase 2 hasn't built yet — the packer below drops it from what's actually
 * rendered (LANE brief: "omitted with a coming soon note in preview only,
 * never a broken card") while the gallery/preview still lists it by name.
 */
import { useEffect, useState } from 'react';
import { apiFetch } from '../../lib/api';
import { reflow, type DashboardCardLayout, type DashboardLayoutData } from './useLayout';

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
    name: 'Morning Briefing',
    question: 'What does today need from me?',
    cards: [
      // h:2 — the manifest 'compact' layout (icon+label, a hero value, an
      // inline supporting row) clipped its hero value's glyphs at h:1.
      { type: 'henry_status', id: 'henry_status', position: { x: 0, y: 0 }, size: { w: 12, h: 2 } },
      { type: 'decisions', id: 'decisions', position: { x: 0, y: 2 }, size: { w: 5, h: 4 } },
      { type: 'today_focus', id: 'today_focus', position: { x: 5, y: 2 }, size: { w: 7, h: 4 } },
      { type: 'in_flight', id: 'in_flight', position: { x: 0, y: 6 }, size: { w: 12, h: 3 } },
      // Calendar at its medium size (7 columns): the full Day / Week / Month
      // calendar, with the others stacked beside it.
      { type: 'calendar', id: 'calendar', position: { x: 0, y: 9 }, size: { w: 7, h: 8 } },
      { type: 'todos', id: 'todos', position: { x: 7, y: 9 }, size: { w: 5, h: 4 } },
      { type: 'overnight_digest', id: 'overnight_digest', position: { x: 7, y: 13 }, size: { w: 5, h: 4 } },
      { type: 'weather', id: 'weather', position: { x: 0, y: 17 }, size: { w: 3, h: 2 } },
      { type: 'system_stats', id: 'system_stats', position: { x: 3, y: 17 }, size: { w: 3, h: 2 } },
    ],
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

/**
 * Live templates from the daemon, falling back to the static mirror above so
 * the gallery still renders with the daemon unreachable (offline, or the
 * harness). Fetched once — templates don't change at runtime.
 */
export function useDashboardTemplates(): DashboardTemplate[] {
  const [templates, setTemplates] = useState<DashboardTemplate[]>(DASHBOARD_TEMPLATES);
  useEffect(() => {
    let cancelled = false;
    apiFetch<DashboardTemplate[]>('/api/dashboard/templates')
      .then(result => { if (!cancelled && Array.isArray(result) && result.length > 0) setTemplates(result); })
      .catch(() => { /* keep the static mirror */ });
    return () => { cancelled = true; };
  }, []);
  return templates;
}
