/**
 * Pending pedestal→tab navigation (C2 nav-honesty) — timer-semantics tests.
 *
 * The invariant under test: a manual navigation during the 700ms pedestal
 * glide can NEVER later yank the user onto the pedestal's tab. Every workspace
 * stays mounted (display:none), so unmount cleanup never runs in the app —
 * cancellation must come from visibility loss (workspace switch / overlay
 * open) and from the store re-check at fire time.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import {
  createPedestalNavController,
  worldNavAllowed,
  PEDESTAL_NAV_DELAY_MS,
} from './pedestalNav';
import type { WorkspaceState } from '../../lib/store';

beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); });

describe('createPedestalNavController', () => {
  it('lands on the tab after the glide when the World stayed visible', () => {
    const navigate = vi.fn();
    const c = createPedestalNavController(navigate);
    c.schedule('build');
    expect(c.hasPending()).toBe(true);
    vi.advanceTimersByTime(PEDESTAL_NAV_DELAY_MS);
    expect(navigate).toHaveBeenCalledWith('build');
    expect(c.hasPending()).toBe(false);
  });

  it('going invisible mid-glide (workspace switch / overlay open) cancels the pending landing', () => {
    const navigate = vi.fn();
    const c = createPedestalNavController(navigate);
    c.schedule('memory');
    vi.advanceTimersByTime(300);
    c.setVisible(false); // Cmd+2 / sidebar click / overlay — workspace div hides
    expect(c.hasPending()).toBe(false);
    vi.advanceTimersByTime(PEDESTAL_NAV_DELAY_MS);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('does not fire while invisible even if the timer somehow survives (belt over braces)', () => {
    const navigate = vi.fn();
    const c = createPedestalNavController(navigate);
    c.schedule('build');
    // Simulate visibility flipping without the clear having landed yet: the
    // fire-time check must still refuse.
    c.setVisible(false);
    c.setVisible(true);
    c.schedule('build');
    c.setVisible(false);
    vi.advanceTimersByTime(PEDESTAL_NAV_DELAY_MS);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('re-checks the store predicate at fire time (ResizeObserver-latency race)', () => {
    // The user navigates in the final ms of the glide: the store already says
    // "not on the World" but the ResizeObserver-driven setVisible(false) has
    // not landed yet — the predicate must veto the landing.
    const navigate = vi.fn();
    let allowed = true;
    const c = createPedestalNavController(navigate, { canNavigate: () => allowed });
    c.schedule('automate');
    vi.advanceTimersByTime(PEDESTAL_NAV_DELAY_MS - 1);
    allowed = false;
    vi.advanceTimersByTime(1);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('a new station click replaces the pending landing (last click wins)', () => {
    const navigate = vi.fn();
    const c = createPedestalNavController(navigate);
    c.schedule('build');
    vi.advanceTimersByTime(400);
    c.schedule('memory');
    vi.advanceTimersByTime(PEDESTAL_NAV_DELAY_MS);
    expect(navigate).toHaveBeenCalledTimes(1);
    expect(navigate).toHaveBeenCalledWith('memory');
  });

  it('cancel() and dispose() clear the pending landing (forum-portal click / unmount)', () => {
    const navigate = vi.fn();
    const c = createPedestalNavController(navigate);
    c.schedule('build');
    c.cancel();
    vi.advanceTimersByTime(PEDESTAL_NAV_DELAY_MS);
    c.schedule('build');
    c.dispose();
    vi.advanceTimersByTime(PEDESTAL_NAV_DELAY_MS);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('becoming visible again does NOT resurrect a cancelled landing', () => {
    const navigate = vi.fn();
    const c = createPedestalNavController(navigate);
    c.schedule('build');
    c.setVisible(false);
    c.setVisible(true); // user comes straight back to the World
    vi.advanceTimersByTime(PEDESTAL_NAV_DELAY_MS * 2);
    expect(navigate).not.toHaveBeenCalled();
  });
});

describe('worldNavAllowed — synchronous store truth for the fire-time check', () => {
  const worldWs = {
    id: 'w1', name: 'World', icon: '', sortOrder: 0, isDefault: false,
    layoutJson: { type: 'panel', tool: 'world' },
  } as unknown as WorkspaceState;
  const buildWs = {
    id: 'w2', name: 'Build', icon: '', sortOrder: 1, isDefault: false,
    layoutJson: { type: 'split', direction: 'horizontal', sizes: [50, 50], children: [
      { type: 'panel', tool: 'terminal' }, { type: 'panel', tool: 'build' },
    ] },
  } as unknown as WorkspaceState;

  it('allows only when the active workspace hosts the World and no overlay is open', () => {
    expect(worldNavAllowed({ activePanel: 'chat', workspaces: [worldWs, buildWs], activeWorkspaceId: 'w1' })).toBe(true);
  });

  it('vetoes when another workspace is active', () => {
    expect(worldNavAllowed({ activePanel: 'chat', workspaces: [worldWs, buildWs], activeWorkspaceId: 'w2' })).toBe(false);
  });

  it('vetoes when an overlay covers the workspace (navigateToTool would close it)', () => {
    expect(worldNavAllowed({ activePanel: 'trace', workspaces: [worldWs], activeWorkspaceId: 'w1' })).toBe(false);
    expect(worldNavAllowed({ activePanel: 'sessions', workspaces: [worldWs], activeWorkspaceId: 'w1' })).toBe(false);
  });

  it('vetoes when no workspace is active', () => {
    expect(worldNavAllowed({ activePanel: 'chat', workspaces: [worldWs], activeWorkspaceId: null })).toBe(false);
  });
});
