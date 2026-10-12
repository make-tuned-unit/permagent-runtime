// "Open on launch" preference — where Permagent lands when the app opens.
//
// LIVE (2026-08): persisted to localStorage here, written by Settings →
// Preferences, and consumed once by App.tsx after workspaces load. The option
// list is deliberately honest — only destinations that actually exist as
// seeded workspace tools (navigateToTool targets).
//
// The contract is what the label says: 'default' is HOME, every launch.
// 'last' is the explicit "pick up where I left off" choice and restores the
// daemon's persisted active workspace (the store loads that on boot). Before
// this the default silently behaved like 'last' while reading "Home (default)".

import type { ToolType } from './store';

const KEY = 'permagent-open-on-launch';

export type OpenOnLaunch = 'default' | 'last' | Extract<ToolType,
  'dashboard' | 'build' | 'memory' | 'projects' | 'people' | 'world' | 'automate' | 'grow' | 'finance'>;

export const OPEN_ON_LAUNCH_OPTIONS: Array<{ value: OpenOnLaunch; label: string }> = [
  { value: 'default', label: 'Home (default)' },
  { value: 'last', label: 'Last workspace' },
  { value: 'dashboard', label: 'Dashboard' },
  { value: 'build', label: 'Build' },
  { value: 'memory', label: 'Brain' },
  { value: 'projects', label: 'Projects' },
  { value: 'people', label: 'People' },
  { value: 'world', label: 'World' },
  { value: 'automate', label: 'Automate' },
  { value: 'grow', label: 'Grow' },
  { value: 'finance', label: 'Finance' },
];

export function getOpenOnLaunch(): OpenOnLaunch {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw && OPEN_ON_LAUNCH_OPTIONS.some(o => o.value === raw)) {
      return raw as OpenOnLaunch;
    }
  } catch { /* storage unavailable — default */ }
  return 'default';
}

export function setOpenOnLaunch(value: OpenOnLaunch): void {
  try {
    localStorage.setItem(KEY, value);
  } catch { /* storage unavailable — the select still reflects the session */ }
}

interface LaunchWorkspace { id: string; isDefault: boolean; sortOrder: number }

/** Home: the workspace the daemon marks default, else the first in sidebar order. */
export function homeWorkspaceId(workspaces: LaunchWorkspace[]): string | null {
  const home = workspaces.find(w => w.isDefault)
    ?? [...workspaces].sort((a, b) => a.sortOrder - b.sortOrder)[0];
  return home?.id ?? null;
}

/** Apply the launch preference once workspaces have loaded. */
export function applyOpenOnLaunch(
  pref: OpenOnLaunch,
  state: { workspaces: LaunchWorkspace[]; activeWorkspaceId: string | null },
  actions: { switchWorkspace: (id: string) => void; navigateToTool: (tool: ToolType) => boolean },
): void {
  if (pref === 'last') return;
  if (pref === 'default') {
    const home = homeWorkspaceId(state.workspaces);
    if (home && home !== state.activeWorkspaceId) actions.switchWorkspace(home);
    return;
  }
  actions.navigateToTool(pref);
}
