import { create } from 'zustand';

/** Navigation only. No recording, permission request, job, write or model call. */
export const PROJECT_LENSES = ['overview', 'details', 'kanban'] as const;
export const WORLD_PLACE_IDS = ['commons', 'gallery', 'maker', 'council_garden', 'observatory_garden', 'conservatory'] as const;
/** Bundled code cards plus daemon built-in manifests; never arbitrary skill cards. */
export const HOME_CARD_TYPES = [
  'decisions',
  'stats',
  'in_flight',
  'recent',
  'timeline',
  'todos',
  'growth_results',
  'council',
  'calendar',
  'today_focus',
  'overnight_digest',
  'fleet_kpis',
  'stalled_pattern',
  'project_health_grid',
  'system_stats',
  'weather',
  'henry_status',
  'job_health',
  'finance_summary',
  'finance_watchlist',
  'cost_vs_outcome',
  'people_attention',
  'open_commitments',
  'recent_meetings',
  'week_headline',
  'week_small_multiples',
  'memory_growth',
  'on_this_week',
] as const;
export type HomeFocusRequest = { section: 'overview' | typeof HOME_CARD_TYPES[number] };
export function isHomeSection(section: string): section is HomeFocusRequest['section'] {
  return section === 'overview' || (HOME_CARD_TYPES as readonly string[]).includes(section);
}
export type ProjectLensRequest = { projectId: string; lens: typeof PROJECT_LENSES[number] };
export type WorldPlaceRequest = { placeId: typeof WORLD_PLACE_IDS[number] };
type RecordOpenRequest = object;

export const useAppNavigationRequests = create<{
  projectLens: ProjectLensRequest | null;
  worldPlace: WorldPlaceRequest | null;
  recordOpen: RecordOpenRequest | null;
  homeFocus: HomeFocusRequest | null;
}>(() => ({ projectLens: null, worldPlace: null, recordOpen: null, homeFocus: null }));

export function requestProjectLens(projectId: string, lens: string): boolean {
  if (!projectId.trim() || !(PROJECT_LENSES as readonly string[]).includes(lens)) return false;
  useAppNavigationRequests.setState({ projectLens: { projectId, lens: lens as ProjectLensRequest['lens'] } });
  return true;
}
export function requestWorldPlace(placeId: string): boolean {
  if (!(WORLD_PLACE_IDS as readonly string[]).includes(placeId)) return false;
  useAppNavigationRequests.setState({ worldPlace: { placeId: placeId as WorldPlaceRequest['placeId'] } });
  return true;
}
export function requestHomeFocus(section: string): boolean {
  if (!isHomeSection(section)) return false;
  useAppNavigationRequests.setState({ homeFocus: { section } });
  return true;
}
export function requestRecordOpen(): void {
  useAppNavigationRequests.setState({ recordOpen: {} });
}
/** Consume only the request this render handled; never clear a newer request. */
export function consumeNavigationRequest<K extends 'projectLens' | 'worldPlace' | 'recordOpen' | 'homeFocus'>(
  key: K, request: ReturnType<typeof useAppNavigationRequests.getState>[K],
): void {
  useAppNavigationRequests.setState(s => s[key] === request ? { [key]: null } : {});
}
