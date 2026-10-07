/** @vitest-environment jsdom */
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
vi.mock('@react-three/fiber', () => ({ Canvas: () => null }));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Ada' }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./forumQuery', () => ({ askFromForum: vi.fn() }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: false }) }));
import { ForumView } from './ForumView';
import { useCommandCenter } from '../../../lib/store';
let root: Root, host: HTMLDivElement;
const end = vi.fn(), handsFree = vi.fn();
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
beforeEach(() => {
  window.history.replaceState({}, '', '/'); end.mockReset(); handsFree.mockReset();
  useCommandCenter.setState({ chatWindowOpen: false, worldConversationActive: false, chatDockOpen: true, voiceEngine: { state: 'playing', handsFree: true, deactivate: end, setHandsFree: handsFree } as unknown as NonNullable<ReturnType<typeof useCommandCenter.getState>['voiceEngine']> });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  act(() => root.render(<ForumView />));
});
afterEach(() => { act(() => root.unmount()); host.remove(); useCommandCenter.setState({ voiceEngine: null, worldConversationActive: false }); });
const click = (label: string) => act(() => [...host.querySelectorAll('button')].find(b => b.textContent?.includes(label))!.click());
it('takes over an existing voice chat at the table without starting a new session', () => {
  click('Sit with Ada');
  expect(useCommandCenter.getState().worldConversationActive).toBe(true);
  expect(useCommandCenter.getState().chatDockOpen).toBe(false);
  expect(host.querySelector('[aria-label="One on one with Ada"]')).toBeTruthy();
  expect(handsFree).not.toHaveBeenCalled(); expect(end).not.toHaveBeenCalled();
});
it('hands active voice controls back to chat when the World is hidden', () => {
  click('Sit with Ada'); act(() => root.render(<ForumView visible={false} />));
  expect(useCommandCenter.getState().worldConversationActive).toBe(false);
  expect(useCommandCenter.getState().chatDockOpen).toBe(true);
  expect(end).not.toHaveBeenCalled();
});
it('explicitly leaving the table ends voice and restores garden controls', () => {
  click('Sit with Ada'); click('Leave table');
  expect(end).toHaveBeenCalledOnce(); expect(handsFree).toHaveBeenCalledWith(false);
  expect(useCommandCenter.getState().worldConversationActive).toBe(false);
  expect(host.querySelector('[aria-label="Agent movement and work"]')).toBeTruthy();
});

it('keeps movement on demand and reading the work feed does not move the camera', () => {
  expect(host.querySelector('#habitat-movement')).toBeNull();
  click('Work feed');
  expect(host.querySelector('#habitat-work-feed')).toBeTruthy();
  click('Movement');
  const follow = [...host.querySelectorAll('button')].find(b => b.textContent === 'Follow')!;
  expect(follow.getAttribute('aria-pressed')).toBe('false');
  click('Move agent');
  expect(host.querySelector('.habitat-destinations')).toBeTruthy();
  click('Movement');
  expect(host.querySelector('.habitat-destinations')).toBeNull();
  expect(host.querySelector('#habitat-movement')).toBeNull();
});
it('keeps the evidence exercise reachable from the consolidated agent desk', () => {
  click('Agent desk'); click('Evidence');
  expect(host.querySelector('#forum-agent-desk')?.getAttribute('aria-hidden')).toBe('false');
  expect(host.querySelector('[aria-label="Completion evidence demonstration"]')).toBeTruthy();
  click('Garden room confirmed');
  expect(host.textContent).toContain('Supported by the records.');
  expect(host.querySelector('.habitat-view-tools')?.textContent).not.toContain('Inspect evidence');
});
