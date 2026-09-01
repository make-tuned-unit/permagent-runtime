/**
 * The "take me to the canonical Decisions surface" seam (J3).
 *
 * Settings → Autonomy used to build its own summary sentence and open its own
 * copy of the inbox overlay. It now says the same sentence Home says and hands
 * the user to Home, which is where the count lives.
 *
 * The pin that matters is the one `focusWorldAgent` already established: arm
 * the pending open ONLY if the destination is reachable. A flag left set with
 * nowhere to go sits in the store and yanks a later, unrelated visit.
 *
 * `./api` is mocked (the chatStop pattern) so importing the store touches no
 * network; with no workspaces loaded, navigateToTool is a no-op.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';

vi.mock('./api', () => ({
  // The workspace switch persists the active tab; the seam under test only
  // cares that it was reachable, so the write resolves and is ignored.
  api: { setActiveWorkspace: vi.fn(async () => undefined) },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

import { useCommandCenter } from './store';

const HOME_WORKSPACE = {
  id: 'ws-home',
  name: 'Home',
  layoutJson: { type: 'panel' as const, tool: 'dashboard' as const },
};

beforeEach(() => {
  useCommandCenter.setState({ pendingDecisionInbox: false, workspaces: [] });
});

describe('openDecisionInbox seam', () => {
  it('refuses, and says so, when no workspace holds Home', () => {
    expect(useCommandCenter.getState().openDecisionInbox()).toBe(false);
    expect(useCommandCenter.getState().pendingDecisionInbox).toBe(false);
  });

  it('arms the open when Home is reachable', () => {
    useCommandCenter.setState({ workspaces: [HOME_WORKSPACE] as never });
    expect(useCommandCenter.getState().openDecisionInbox()).toBe(true);
    expect(useCommandCenter.getState().pendingDecisionInbox).toBe(true);
  });

  it('clears once the canonical card has opened, so it cannot fire twice', () => {
    useCommandCenter.setState({ workspaces: [HOME_WORKSPACE] as never });
    useCommandCenter.getState().openDecisionInbox();
    useCommandCenter.getState().clearPendingDecisionInbox();
    expect(useCommandCenter.getState().pendingDecisionInbox).toBe(false);
  });
});
