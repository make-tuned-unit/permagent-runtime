import {
  DEFAULT_LAYOUT, LEGACY_DEFAULT_LAYOUT, LEGACY_MORNING_LAYOUT, LEGACY_MORNING_TEMPLATE_LAYOUT, LEGACY_DAEMON_LAYOUTS, reflow, stripRetiredCards, compactLayoutPass, ensureGrowthResultsCard,
  ensureGrowthCardTallEnough, ensureCalendarCard, ensureCouncilCard,
  type DashboardLayoutData,
} from './useLayout';

// Recognize only exact shipped layouts, including the daemon's older defaults
// after the existing migrations. A renamed, resized, hidden or reordered card
// is a user's layout and must remain visible even on an otherwise empty profile.
function signature(layout: DashboardLayoutData): string {
  return JSON.stringify(layout.cards.map(c => [c.id, c.type, c.position.x, c.position.y, c.size.w, c.size.h, c.visible]));
}

const defaultSignatures = new Set<string>();
for (const initial of [DEFAULT_LAYOUT, LEGACY_DEFAULT_LAYOUT, LEGACY_MORNING_LAYOUT, LEGACY_MORNING_TEMPLATE_LAYOUT,
  { cards: reflow(LEGACY_MORNING_TEMPLATE_LAYOUT.cards) }, ...LEGACY_DAEMON_LAYOUTS]) {
  let layout = initial;
  defaultSignatures.add(signature(layout));
  for (const migrate of [stripRetiredCards, compactLayoutPass, ensureGrowthResultsCard, ensureGrowthCardTallEnough, ensureCalendarCard, ensureCouncilCard]) {
    layout = migrate(layout).layout;
    defaultSignatures.add(signature(layout));
  }
}

export function isDefaultHomeLayout(layout: DashboardLayoutData): boolean {
  return defaultSignatures.has(signature(layout));
}

/**
 * Whether the dashboard's own numbers describe a profile nobody has used yet.
 * The daemon's `sessions_total` counts only sessions with messages (an empty
 * chat dock is not a run) and `memory_count` excludes the four seeded
 * "onboarding" welcome memories, so zero here means genuinely untouched.
 * Empty-looking sessions that still reach the lists are ignored too.
 */
export function isUntouchedHomeData(data: {
  stats: { sessions_total: number; memory_count: number };
  recent: unknown[];
  in_flight: unknown[];
}): boolean {
  return data.stats.sessions_total === 0 && data.stats.memory_count === 0
    && data.recent.length === 0 && data.in_flight.length === 0;
}
