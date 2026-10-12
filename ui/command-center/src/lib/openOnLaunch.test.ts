/** @vitest-environment jsdom */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { applyOpenOnLaunch, getOpenOnLaunch, homeWorkspaceId, setOpenOnLaunch, OPEN_ON_LAUNCH_OPTIONS } from './openOnLaunch';

const workspaces = [
  { id: 'build', isDefault: false, sortOrder: 3 },
  { id: 'home', isDefault: true, sortOrder: 0 },
  { id: 'world', isDefault: false, sortOrder: 7 },
];
const actions = () => ({ switchWorkspace: vi.fn(), navigateToTool: vi.fn(() => true) });

beforeEach(() => localStorage.clear());

describe('open on launch contract', () => {
  it('with no stored choice the preference is default (Home)', () => {
    expect(getOpenOnLaunch()).toBe('default');
    expect(OPEN_ON_LAUNCH_OPTIONS[0]).toEqual({ value: 'default', label: 'Home (default)' });
  });

  it('default lands on Home even when the daemon restored another workspace', () => {
    const a = actions();
    applyOpenOnLaunch('default', { workspaces, activeWorkspaceId: 'build' }, a);
    expect(a.switchWorkspace).toHaveBeenCalledWith('home');
  });

  it('default does nothing when already on Home', () => {
    const a = actions();
    applyOpenOnLaunch('default', { workspaces, activeWorkspaceId: 'home' }, a);
    expect(a.switchWorkspace).not.toHaveBeenCalled();
  });

  it('"last" restores the actual last workspace: no switching at all', () => {
    const a = actions();
    setOpenOnLaunch('last');
    expect(getOpenOnLaunch()).toBe('last');
    applyOpenOnLaunch(getOpenOnLaunch(), { workspaces, activeWorkspaceId: 'build' }, a);
    expect(a.switchWorkspace).not.toHaveBeenCalled();
    expect(a.navigateToTool).not.toHaveBeenCalled();
  });

  it('an explicit tool goes there', () => {
    const a = actions();
    applyOpenOnLaunch('world', { workspaces, activeWorkspaceId: 'home' }, a);
    expect(a.navigateToTool).toHaveBeenCalledWith('world');
  });

  it('Home falls back to the first workspace in sidebar order when none is marked default', () => {
    expect(homeWorkspaceId(workspaces.map(w => ({ ...w, isDefault: false })))).toBe('home');
    expect(homeWorkspaceId([])).toBeNull();
  });
});
