/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

// MainContent's empty/error branches do not mount the application surfaces.
// Keep this test focused on the shell state without importing the terminal and
// 3D workspace implementations into jsdom.
vi.mock('./components/sidebar/Sidebar', () => ({ Sidebar: () => null }));
vi.mock('./components/settings/SettingsView', () => ({ SettingsView: () => null }));
vi.mock('./components/skills/SkillsPanel', () => ({ SkillsPanel: () => null }));
vi.mock('./components/workspaces/WorkspaceRenderer', () => ({ WorkspaceRenderer: () => null }));
vi.mock('./components/workspaces/WorkspaceSaveErrorChip', () => ({ WorkspaceSaveErrorChip: () => null }));
vi.mock('./components/wizard/WizardShell', () => ({ WizardShell: () => null }));
vi.mock('./components/splash/Splash', () => ({ Splash: () => null }));
vi.mock('./components/chat/ChatLauncher', () => ({ ChatLauncher: () => null }));
vi.mock('./components/chat/ChatDock', () => ({ ChatDock: () => null }));
vi.mock('./components/voice/VoiceHost', () => ({ VoiceHost: () => null }));
vi.mock('./components/goals/GoalDetailModal', () => ({ GoalDetailModalHost: () => null }));
vi.mock('./components/projects/PersonDetailModal', () => ({ PersonDetailModalHost: () => null }));
vi.mock('./components/chat/DropZone', () => ({ DropZone: ({ children }: { children: unknown }) => children }));
vi.mock('./components/notifications/NotificationHost', () => ({ NotificationHost: () => null }));
vi.mock('./components/version/VersionSkewBanner', () => ({ VersionSkewBanner: () => null }));
vi.mock('./hooks/useAppNavigate', () => ({ useAppNavigate: () => undefined }));
vi.mock('./hooks/useVersionSkew', () => ({ useVersionSkew: () => ({}) }));
vi.mock('./lib/repaintOnRegain', () => ({ onRepaintRegain: () => () => {}, forceCompositorRepaint: () => {} }));

import { MainContent } from './App';
import { useCommandCenter } from './lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useCommandCenter.setState({
    activePanel: 'chat',
    activeWorkspaceId: null,
    workspaces: [],
    workspacesLoaded: true,
    workspacesError: false,
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render() {
  act(() => root.render(<MainContent />));
}

describe('MainContent workspace load states', () => {
  it('renders an explicit retry state when workspace loading fails', () => {
    const retry = vi.fn();
    useCommandCenter.setState({ workspacesError: true, loadWorkspaces: retry });
    render();

    expect(container.textContent).toContain("Couldn't load workspaces.");
    expect(container.textContent).toContain('not an empty workspace list');
    const button = Array.from(container.querySelectorAll('button')).find(
      candidate => candidate.textContent === 'Try again',
    );
    expect(button).toBeDefined();
    act(() => { button!.click(); });
    expect(retry).toHaveBeenCalledTimes(1);
  });

  it('keeps a genuine empty workspace response distinct from offline failure', () => {
    render();

    expect(container.textContent).toContain('No workspaces available');
    expect(container.textContent).not.toContain('not an empty workspace list');
    expect(container.querySelector('button')).toBeNull();
  });
});
