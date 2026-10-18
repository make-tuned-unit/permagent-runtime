/** @vitest-environment jsdom */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';

const noticeState = vi.hoisted(() => ({
  items: [] as Array<{ id: string; kind: 'system'; title: string; body: string; ts: number; read: boolean; target?: 'dashboard' | 'build'; url?: string }>,
  trayOpen: true,
  markRead: vi.fn(),
}));
vi.mock('../../lib/notifications', async importOriginal => {
  const actual = await importOriginal<typeof import('../../lib/notifications')>();
  return {
    ...actual,
    useNotifications: () => ({ items: noticeState.items }),
    markRead: (...args: unknown[]) => noticeState.markRead(...args),
    markAllRead: vi.fn(),
    setTrayOpen: (open: boolean) => { noticeState.trayOpen = open; },
    useTrayOpen: () => noticeState.trayOpen,
  };
});
vi.mock('../dashboard/decisions/useDecisions', () => ({
  useDecisions: () => ({ data: null, loading: true, error: false }),
}));

import { NotificationHost } from './NotificationHost';
import { useCommandCenter } from '../../lib/store';
import type { LayoutNode, WorkspaceState } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const workspace = (id: string, tool: 'dashboard' | 'build' | 'people'): WorkspaceState => ({
  id, name: id, icon: '', sortOrder: 0,
  layoutJson: { type: 'panel', tool, config: {} } satisfies LayoutNode,
  isDefault: false,
});
let host: HTMLDivElement;
let root: Root;
let original: ReturnType<typeof useCommandCenter.getState>;

beforeEach(() => {
  original = useCommandCenter.getState();
  noticeState.items = [{ id: 'notification-1', kind: 'system', title: 'Open destination', body: '', ts: Date.now(), read: false, target: 'dashboard' }];
  noticeState.trayOpen = true;
  noticeState.markRead.mockReset();
  useCommandCenter.setState({
    workspaces: [workspace('current', 'people'), workspace('home', 'dashboard'), workspace('build-workspace', 'build')],
    activeWorkspaceId: 'current', activePanel: 'settings', pendingBrowserUrl: null,
    switchWorkspace: (id: string) => useCommandCenter.setState({ activeWorkspaceId: id }),
  });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  noticeState.trayOpen = false;
  useCommandCenter.setState({
    workspaces: original.workspaces,
    activeWorkspaceId: original.activeWorkspaceId,
    activePanel: original.activePanel,
    pendingBrowserUrl: original.pendingBrowserUrl,
    switchWorkspace: original.switchWorkspace,
  });
});

async function render() {
  await act(async () => { root.render(<NotificationHost />); });
}
async function activateRow() {
  const row = host.querySelector<HTMLButtonElement>('[data-notifications-ui] button:not(.pa-btn)')!;
  await act(async () => { row.click(); });
}

it('routes a notification target before acknowledging it', async () => {
  await render();
  await activateRow();

  expect(useCommandCenter.getState().activeWorkspaceId).toBe('home');
  expect(useCommandCenter.getState().activePanel).toBe('chat');
  expect(noticeState.markRead).toHaveBeenCalledWith('notification-1');
  expect(noticeState.trayOpen).toBe(false);
});

it('keeps an unreachable notification unread in the open tray', async () => {
  noticeState.items[0].target = 'dashboard';
  useCommandCenter.setState({ workspaces: [workspace('current', 'people')] });
  await render();
  await activateRow();

  expect(noticeState.markRead).not.toHaveBeenCalled();
  expect(noticeState.trayOpen).toBe(true);
});

it('keeps an unreachable target toast visible instead of dismissing it as successful', async () => {
  noticeState.trayOpen = false;
  useCommandCenter.setState({ workspaces: [workspace('current', 'people')] });
  await render();
  const toast = host.querySelector<HTMLButtonElement>('[role="status"] button')!;
  await act(async () => { toast.click(); });

  expect(noticeState.markRead).not.toHaveBeenCalled();
  expect(host.querySelector('[role="status"]')).toBeTruthy();
});

it('opens a linked notification in the Build browser and then acknowledges it', async () => {
  noticeState.items[0] = { ...noticeState.items[0], target: undefined, url: 'https://example.test/story' };
  await render();
  await activateRow();

  expect(useCommandCenter.getState().activeWorkspaceId).toBe('build-workspace');
  expect(useCommandCenter.getState().pendingBrowserUrl).toBe('https://example.test/story');
  expect(noticeState.markRead).toHaveBeenCalledWith('notification-1');
  expect(noticeState.trayOpen).toBe(false);
});

it('does not queue or acknowledge a linked notification when Build is unreachable', async () => {
  noticeState.items[0] = { ...noticeState.items[0], target: undefined, url: 'https://example.test/story' };
  useCommandCenter.setState({ workspaces: [workspace('current', 'people')], pendingBrowserUrl: null });
  await render();
  await activateRow();

  expect(useCommandCenter.getState().pendingBrowserUrl).toBeNull();
  expect(noticeState.markRead).not.toHaveBeenCalled();
  expect(noticeState.trayOpen).toBe(true);
});
