// "Open on launch" preference — where Permagent lands when the app opens.
//
// LIVE (2026-08): persisted to localStorage here, written by Settings →
// Preferences, and consumed once by App.tsx after workspaces load. The option
// list is deliberately honest — only destinations that actually exist as
// seeded workspace tools (navigateToTool targets). 'default' means "do
// nothing": the app keeps its existing behavior (default workspace + chat).

import type { ToolType } from './store';

const KEY = 'permagent-open-on-launch';

export type OpenOnLaunch = 'default' | Extract<ToolType,
  'dashboard' | 'build' | 'memory' | 'projects' | 'world' | 'automate' | 'grow'>;

export const OPEN_ON_LAUNCH_OPTIONS: Array<{ value: OpenOnLaunch; label: string }> = [
  { value: 'default', label: 'Home (default)' },
  { value: 'dashboard', label: 'Dashboard' },
  { value: 'build', label: 'Build' },
  { value: 'memory', label: 'Brain' },
  { value: 'projects', label: 'Projects' },
  { value: 'world', label: 'World' },
  { value: 'automate', label: 'Automate' },
  { value: 'grow', label: 'Grow' },
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
