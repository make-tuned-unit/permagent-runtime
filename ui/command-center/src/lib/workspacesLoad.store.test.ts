/**
 * Workspace load-state honesty. A failed initial fetch used to set only
 * `workspacesLoaded`, so MainContent rendered the same "No workspaces
 * available" state as a successful empty daemon response.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';

const { getWorkspaces, getActiveWorkspace } = vi.hoisted(() => ({
  getWorkspaces: vi.fn(),
  getActiveWorkspace: vi.fn(),
}));

vi.mock('./api', () => ({
  api: { getWorkspaces, getActiveWorkspace },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

import { useCommandCenter } from './store';

const workspace = {
  id: 'w1',
  name: 'Build',
  icon: 'terminal',
  sortOrder: 0,
  layoutJson: { type: 'panel', tool: 'terminal' },
  isDefault: true,
};

beforeEach(() => {
  getWorkspaces.mockReset();
  getActiveWorkspace.mockReset();
  useCommandCenter.setState({
    workspaces: [],
    activeWorkspaceId: null,
    workspacesLoaded: false,
    workspacesError: false,
  });
});

describe('loadWorkspaces error latch', () => {
  it('keeps the last-known list and latches an error when the daemon is offline', async () => {
    useCommandCenter.setState({
      workspaces: [{ ...workspace, layoutJson: workspace.layoutJson as never }],
      activeWorkspaceId: 'w1',
      workspacesLoaded: true,
    });
    getWorkspaces.mockRejectedValue(new Error('ECONNREFUSED'));
    getActiveWorkspace.mockRejectedValue(new Error('ECONNREFUSED'));

    await useCommandCenter.getState().loadWorkspaces();

    const state = useCommandCenter.getState();
    expect(state.workspaces).toHaveLength(1);
    expect(state.activeWorkspaceId).toBe('w1');
    expect(state.workspacesLoaded).toBe(true);
    expect(state.workspacesError).toBe(true);
  });

  it('keeps a genuinely empty daemon response separate from an error', async () => {
    getWorkspaces.mockResolvedValue([]);
    getActiveWorkspace.mockResolvedValue({ workspaceId: null });

    await useCommandCenter.getState().loadWorkspaces();

    const state = useCommandCenter.getState();
    expect(state.workspaces).toEqual([]);
    expect(state.activeWorkspaceId).toBeNull();
    expect(state.workspacesLoaded).toBe(true);
    expect(state.workspacesError).toBe(false);
  });

  it('clears the error and restores the workspace on a successful retry', async () => {
    getWorkspaces.mockRejectedValueOnce(new Error('daemon down'));
    getActiveWorkspace.mockRejectedValueOnce(new Error('daemon down'));
    await useCommandCenter.getState().loadWorkspaces();
    expect(useCommandCenter.getState().workspacesError).toBe(true);

    getWorkspaces.mockResolvedValueOnce([workspace]);
    getActiveWorkspace.mockResolvedValueOnce({ workspaceId: 'w1' });
    await useCommandCenter.getState().loadWorkspaces();

    const state = useCommandCenter.getState();
    expect(state.workspacesError).toBe(false);
    expect(state.activeWorkspaceId).toBe('w1');
    expect(state.workspaces.map(({ id }) => id)).toEqual(['w1']);
  });
});
