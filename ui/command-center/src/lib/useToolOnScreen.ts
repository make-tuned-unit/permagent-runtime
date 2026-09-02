/**
 * Is this tool actually the one the user is looking at?
 *
 * `App.tsx` renders every workspace at once and hides the inactive ones with
 * `display:none`, so that Terminal and Browser sessions survive a tab switch.
 * The cost is that "mounted" tells a polling hook nothing: a Grow tab the user
 * left twenty minutes ago is still mounted, still running its effects, and
 * `document.visibilityState` still says `visible` because the *window* is.
 *
 * So a poll that wants to be honest about both halves of R1.4 — refresh what
 * is on screen, burn nothing on what is not — needs this: the window is
 * visible AND no overlay is covering the workspaces AND the active workspace
 * is one that hosts this tool. Pair it with `usePollWhenVisible`'s `enabled`
 * argument, which handles the window half and the catch-up on return.
 */

import { useCommandCenter, layoutHasTool, type ToolType } from './store';

export function useToolOnScreen(tool: ToolType): boolean {
  const activePanel = useCommandCenter(s => s.activePanel);
  const activeWorkspaceId = useCommandCenter(s => s.activeWorkspaceId);
  const workspaces = useCommandCenter(s => s.workspaces);

  // Settings and Skills render as full-bleed overlays above the workspaces.
  if (activePanel === 'settings' || activePanel === 'skills') return false;

  const active = workspaces.find(w => w.id === activeWorkspaceId);
  if (!active) return false;
  return layoutHasTool(active.layoutJson, tool);
}
