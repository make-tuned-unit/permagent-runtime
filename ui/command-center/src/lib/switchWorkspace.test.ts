/**
 * switchWorkspace — emission-hygiene tests.
 *
 * The regression this locks down: re-selecting the already-active workspace
 * (a re-click on the current tab, or a daemon-driven AppNavigate to the view
 * the user is already on) must be a full no-op — no re-emitted `*_opened`
 * engagement event, no re-persist of the active id. A genuine switch still
 * emits exactly one open event for instrumented surfaces and none for surfaces
 * that carry their own instrumentation. `./api` is mocked (the chatStop
 * pattern) so no network is touched; `./emitActivity` is mocked to observe
 * emissions.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';

const { emitActivity, setActiveWorkspace } = vi.hoisted(() => ({
  emitActivity: vi.fn(),
  setActiveWorkspace: vi.fn(() => Promise.resolve()),
}));

vi.mock('./emitActivity', () => ({ emitActivity }));
vi.mock('./api', () => ({
  api: { setActiveWorkspace },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

// Imported AFTER the mocks are registered (vi.mock is hoisted above imports).
import { useCommandCenter, type ToolType, type WorkspaceState } from './store';

function workspace(id: string, tool: ToolType): WorkspaceState {
  return {
    id,
    name: id,
    icon: 'o',
    sortOrder: 0,
    layoutJson: { type: 'panel', tool, config: {} },
    isDefault: false,
  };
}

beforeEach(() => {
  emitActivity.mockClear();
  setActiveWorkspace.mockClear();
  useCommandCenter.setState({
    workspaces: [workspace('ws-world', 'world'), workspace('ws-grow', 'grow'), workspace('ws-term', 'terminal')],
    activeWorkspaceId: 'ws-world',
  });
});

describe('switchWorkspace', () => {
  it('re-selecting the active workspace emits nothing and does not re-persist', () => {
    useCommandCenter.getState().switchWorkspace('ws-world');
    expect(emitActivity).not.toHaveBeenCalled();
    expect(setActiveWorkspace).not.toHaveBeenCalled();
    expect(useCommandCenter.getState().activeWorkspaceId).toBe('ws-world');
  });

  it('a genuine switch emits exactly one open event for the target surface', () => {
    useCommandCenter.getState().switchWorkspace('ws-grow');
    expect(useCommandCenter.getState().activeWorkspaceId).toBe('ws-grow');
    expect(setActiveWorkspace).toHaveBeenCalledWith('ws-grow');
    expect(emitActivity).toHaveBeenCalledTimes(1);
    expect(emitActivity).toHaveBeenCalledWith('grow_opened', 'grow');
  });

  it('switching to a self-instrumented surface (terminal) switches without emitting an open event', () => {
    useCommandCenter.getState().switchWorkspace('ws-term');
    expect(useCommandCenter.getState().activeWorkspaceId).toBe('ws-term');
    expect(setActiveWorkspace).toHaveBeenCalledWith('ws-term');
    expect(emitActivity).not.toHaveBeenCalled();
  });

  it('switch away then back emits again — both are real navigations', () => {
    useCommandCenter.getState().switchWorkspace('ws-grow');
    useCommandCenter.getState().switchWorkspace('ws-world');
    expect(emitActivity).toHaveBeenCalledTimes(2);
    expect(emitActivity).toHaveBeenNthCalledWith(1, 'grow_opened', 'grow');
    expect(emitActivity).toHaveBeenNthCalledWith(2, 'world_view_opened', 'world');
  });
});
