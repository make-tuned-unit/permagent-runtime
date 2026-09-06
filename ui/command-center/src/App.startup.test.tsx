/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  getConfig: vi.fn(),
  loadWorkspaces: vi.fn(),
  loadSkills: vi.fn(),
  setActivePanel: vi.fn(),
}));

vi.mock('./lib/api', () => ({
  api: {
    getConfig: mocks.getConfig,
    provisionDictationModel: vi.fn(),
  },
  fileToBase64: vi.fn(),
}));
vi.mock('./lib/store', () => ({
  navigateToTool: vi.fn(),
  useCommandCenter: (selector: (state: Record<string, unknown>) => unknown) => selector({
    activePanel: 'chat',
    activeWorkspaceId: null,
    workspaces: [],
    workspacesLoaded: true,
    workspacesError: false,
    loadWorkspaces: mocks.loadWorkspaces,
    loadSkills: mocks.loadSkills,
    setActivePanel: mocks.setActivePanel,
  }),
}));
vi.mock('./styles/useTheme', () => ({
  useTheme: () => ({
    gradient: { shell: '#10141f', sidebar: '#0b1020' },
    density: 'comfortable',
    theme: 'silver',
    themePref: 'system',
  }),
}));
vi.mock('./components/splash/Splash', () => ({
  Splash: ({ onDone }: { onDone: () => void }) => <button data-testid="splash" onClick={onDone}>Start</button>,
}));
vi.mock('./components/wizard/WizardShell', () => ({ WizardShell: () => <div data-testid="wizard">Wizard</div> }));
vi.mock('./components/common/StateBlock', () => ({
  StateBlock: ({ title, detail, onRetry }: { title: string; detail?: string; onRetry?: () => void }) => (
    <div data-testid="state-block">
      <div>{title}</div>
      <div>{detail}</div>
      {onRetry && <button onClick={onRetry}>Try again</button>}
    </div>
  ),
}));
vi.mock('./components/sidebar/Sidebar', () => ({ Sidebar: () => null }));
vi.mock('./components/settings/SettingsView', () => ({ SettingsView: () => null }));
vi.mock('./components/skills/SkillsPanel', () => ({ SkillsPanel: () => null }));
vi.mock('./components/workspaces/WorkspaceRenderer', () => ({ WorkspaceRenderer: () => null }));
vi.mock('./components/workspaces/WorkspaceSaveErrorChip', () => ({ WorkspaceSaveErrorChip: () => null }));
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
vi.mock('./lib/openOnLaunch', () => ({ getOpenOnLaunch: () => 'default' }));
vi.mock('./lib/chatWindow', () => ({ createChatWindow: vi.fn() }));

import App, { classifyStartupConfig } from './App';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  mocks.getConfig.mockReset();
  mocks.loadWorkspaces.mockReset();
  mocks.loadSkills.mockReset();
  mocks.setActivePanel.mockReset();
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function enterLoading() {
  act(() => root.render(<App />));
  act(() => (container.querySelector('[data-testid="splash"]') as HTMLButtonElement).click());
  await act(async () => { await Promise.resolve(); });
}

describe('startup config contract', () => {
  it('keeps connection and auth failures out of onboarding', async () => {
    vi.useFakeTimers();
    mocks.getConfig.mockRejectedValue(new Error('daemon unavailable'));
    await enterLoading();
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(container.querySelector('[data-testid="state-block"]')?.textContent).toContain('not a first-run state');
    expect(container.querySelector('[data-testid="wizard"]')).toBeNull();
  });

  it('keeps malformed config distinct from first-run config', async () => {
    expect(classifyStartupConfig({ config: null })).toBe('malformed');
    expect(classifyStartupConfig({ config: { wizard_complete: 'false' } })).toBe('malformed');
    expect(classifyStartupConfig({ config: {} })).toBe('wizard');
    expect(classifyStartupConfig({ config: { wizard_complete: false } })).toBe('wizard');
    expect(classifyStartupConfig({ config: { wizard_complete: true } })).toBe('app');
  });

  it('shows the invalid-config recovery state instead of onboarding', async () => {
    mocks.getConfig.mockResolvedValue({ config: null });
    await enterLoading();
    expect(container.querySelector('[data-testid="state-block"]')?.textContent).toContain('invalid configuration');
    expect(container.querySelector('[data-testid="wizard"]')).toBeNull();
  });

  it('enters onboarding only for a valid first-run response', async () => {
    mocks.getConfig.mockResolvedValue({ config: { wizard_complete: false } });
    await enterLoading();
    expect(container.querySelector('[data-testid="wizard"]')).not.toBeNull();
  });

  it('enters the app for a completed response and retries successfully', async () => {
    mocks.getConfig
      .mockRejectedValueOnce(new Error('daemon still starting'))
      .mockResolvedValueOnce({ config: { wizard_complete: true } });
    vi.useFakeTimers();
    await enterLoading();
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(container.querySelector('[data-testid="wizard"]')).toBeNull();
    expect(mocks.loadWorkspaces).toHaveBeenCalled();
    expect(mocks.loadSkills).toHaveBeenCalled();
  });
});
