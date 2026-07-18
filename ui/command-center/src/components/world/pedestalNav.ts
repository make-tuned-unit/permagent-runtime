/**
 * Pending pedestal→tab navigation (C2 nav-honesty fix).
 *
 * Clicking a cardinal pedestal glides the camera for PEDESTAL_NAV_DELAY_MS and
 * then lands the user on the pedestal's product tab. The stated invariant is
 * that a manual navigation during the glide can never yank the user: before
 * this module the pending timer was cleared only on another station click or on
 * WorldView unmount — but App keeps every workspace MOUNTED (`display:none`),
 * so Cmd+N / a sidebar click / opening an overlay within the 700ms window let
 * the timer fire anyway, dragging the user off the surface they had just chosen
 * (navigateToTool also closes any open overlay via setActivePanel('chat')).
 *
 * The controller enforces the invariant twice over:
 *  • `setVisible(false)` — driven by useWorldVisibility (ResizeObserver: the
 *    workspace div gets `display:none` on any workspace switch OR overlay
 *    open) — cancels the pending navigation outright;
 *  • at fire time the callback re-checks visibility AND the injected
 *    `canNavigate` predicate (the store's synchronous truth: World workspace
 *    active, no overlay), closing the ResizeObserver-latency race where the
 *    user navigates in the final milliseconds of the glide.
 *
 * Pure factory + pure predicate so the timer semantics unit-test without
 * three.js or a DOM.
 */

import type { LayoutNode, ToolType, WorkspaceState } from '../../lib/store';

/** The pedestal camera glide plays for this long, then the user lands on the
 *  tab — a "dive toward the zone" transition kept from the watch-only diorama. */
export const PEDESTAL_NAV_DELAY_MS = 700;

export interface PedestalNavController {
  /** Arm (or re-arm — a new click replaces any pending one) the landing. */
  schedule(tool: ToolType): void;
  /** Clear any pending landing. */
  cancel(): void;
  /** Report whether the World is the visibly rendered workspace; turning
   *  invisible cancels any pending landing. */
  setVisible(visible: boolean): void;
  hasPending(): boolean;
  /** Unmount cleanup — same as cancel. */
  dispose(): void;
}

export function createPedestalNavController(
  navigate: (tool: ToolType) => void,
  opts: { delayMs?: number; canNavigate?: () => boolean } = {},
): PedestalNavController {
  const delayMs = opts.delayMs ?? PEDESTAL_NAV_DELAY_MS;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let visible = true;
  const clear = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };
  return {
    schedule(tool) {
      clear();
      timer = setTimeout(() => {
        timer = null;
        // Fire only while the World is still the surface the user is on.
        if (!visible) return;
        if (opts.canNavigate && !opts.canNavigate()) return;
        navigate(tool);
      }, delayMs);
    },
    cancel: clear,
    setVisible(v) {
      visible = v;
      if (!v) clear();
    },
    hasPending: () => timer !== null,
    dispose: clear,
  };
}

function layoutHasWorld(node: LayoutNode): boolean {
  if (node.type === 'panel') return node.tool === 'world';
  if (node.type === 'split') return node.children.some(layoutHasWorld);
  return false;
}

/**
 * Synchronous store truth for "may a pedestal landing fire right now": the
 * active workspace hosts the World AND no overlay panel is covering it. Pure —
 * pass the current store state.
 */
export function worldNavAllowed(state: {
  activePanel: string;
  workspaces: WorkspaceState[];
  activeWorkspaceId: string | null;
}): boolean {
  if (state.activePanel !== 'chat') return false;
  const ws = state.workspaces.find(w => w.id === state.activeWorkspaceId);
  return !!ws && layoutHasWorld(ws.layoutJson);
}
