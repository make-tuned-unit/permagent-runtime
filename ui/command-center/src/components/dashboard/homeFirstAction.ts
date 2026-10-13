import {
  DEFAULT_LAYOUT, stripRetiredCards, compactLayoutPass, ensureGrowthResultsCard,
  ensureGrowthCardTallEnough, ensureCalendarCard, ensureCouncilCard,
  type DashboardLayoutData,
} from './useLayout';

// Recognize only exact shipped layouts, including the daemon's older defaults
// after the existing migrations. A renamed, resized, hidden or reordered card
// is a user's layout and must remain visible even on an otherwise empty profile.
function signature(layout: DashboardLayoutData): string {
  return JSON.stringify(layout.cards.map(c => [c.id, c.type, c.position.x, c.position.y, c.size.w, c.size.h, c.visible]));
}

function daemonDefault(enabled: boolean): DashboardLayoutData {
  const card = (type: string, x: number, y: number, w: number, h: number) =>
    ({ id: type, type, position: { x, y }, size: { w, h }, visible: true });
  return { cards: [
    card('hero', 0, 0, 7, 4),
    ...(enabled ? [card('decisions', 7, 0, 5, 4)] : []),
    card('stats', enabled ? 0 : 7, enabled ? 4 : 0, 5, 4),
    card('in_flight', 0, enabled ? 8 : 4, 12, 3),
    card('recent', 0, enabled ? 11 : 7, 12, 4),
  ] };
}

const defaultSignatures = new Set<string>();
for (const initial of [DEFAULT_LAYOUT, daemonDefault(false), daemonDefault(true)]) {
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
