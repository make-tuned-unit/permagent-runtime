/** @vitest-environment jsdom */
/**
 * The forum page must say when it cannot reach the agent, and offer the fix.
 *
 * 2026-09-19: the standalone forum (localhost:5173/ui/forum.html) in an
 * unpaired tab stopped sending 401s, but `daemonAuthError` was only drawn by
 * the chat's MessageList, which this page never mounts — the World just went
 * quiet. What this proves: ForumView renders the published auth message as a
 * role="alert", offers a "Pair this tab" box that hands the pasted link to
 * `pairThisBrowser`, and the alert goes away when the error clears. What it
 * does not prove: placement or visibility on screen (jsdom has no layout), nor
 * the redemption itself (apiUnpaired.test.ts covers that).
 */
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { createRoot, Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
vi.mock('@react-three/fiber', () => ({ Canvas: () => null }));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Aster' }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./forumQuery', () => ({ askFromForum: vi.fn() }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('./ForumAgents', () => ({ ForumAgents: () => null }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: false }) }));
const pair = vi.hoisted(() => vi.fn());
vi.mock('../../../lib/api', async importOriginal => ({
  ...(await importOriginal<typeof import('../../../lib/api')>()),
  pairThisBrowser: pair,
  canPairThisBrowser: () => true,
}));
import { ForumView } from './ForumView';
import { useCommandCenter } from '../../../lib/store';

let root: Root, host: HTMLDivElement;
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
beforeEach(() => {
  pair.mockReset();
  useCommandCenter.setState({ daemonAuthError: null });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

const alertEl = () => host.querySelector('[role="alert"]');

it('shows nothing extra while the credential is healthy', () => {
  act(() => root.render(<ForumView />));
  expect(alertEl()).toBeNull();
});

it('shows why the World cannot reach the agent, and clears when paired', () => {
  act(() => { useCommandCenter.setState({ daemonAuthError: 'This browser is not paired with the daemon' }); });
  act(() => root.render(<ForumView />));
  act(() => { [...host.querySelectorAll('button')].find(b => b.textContent?.includes('Connect…'))?.click(); });
  expect(alertEl()?.textContent).toContain('Not connected to your agent');
  expect(alertEl()?.textContent).toContain('This browser is not paired with the daemon');
  act(() => { useCommandCenter.setState({ daemonAuthError: null }); });
  expect(alertEl()).toBeNull();
});

it('pairs this tab from a pasted Settings → Devices link', async () => {
  pair.mockResolvedValue({ paired: true, message: null });
  act(() => { useCommandCenter.setState({ daemonAuthError: 'The pairing link could not be redeemed (HTTP 404 from POST /pair/claim)' }); });
  act(() => root.render(<ForumView />));
  act(() => { [...host.querySelectorAll('button')].find(b => b.textContent?.includes('Connect…'))?.click(); });
  const input = host.querySelector('#forum-pair-link') as HTMLInputElement;
  expect(input).toBeTruthy();
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
  act(() => {
    setValue.call(input, 'http://hub:3001/ui/#claim=abc123abc123');
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const button = [...alertEl()!.querySelectorAll('button')].find(b => b.textContent?.includes('Pair this tab'))!;
  expect(button.disabled).toBe(false);
  await act(async () => { button.click(); });
  expect(pair).toHaveBeenCalledWith('http://hub:3001/ui/#claim=abc123abc123');
});

it('says so when the pasted text is not a pairing link', async () => {
  pair.mockResolvedValue({ paired: false, message: 'That is not a pairing link.' });
  act(() => { useCommandCenter.setState({ daemonAuthError: 'not paired' }); });
  act(() => root.render(<ForumView />));
  act(() => { [...host.querySelectorAll('button')].find(b => b.textContent?.includes('Connect…'))?.click(); });
  const input = host.querySelector('#forum-pair-link') as HTMLInputElement;
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
  act(() => {
    setValue.call(input, 'hello');
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const button = [...alertEl()!.querySelectorAll('button')].find(b => b.textContent?.includes('Pair this tab'))!;
  await act(async () => { button.click(); });
  expect(alertEl()?.textContent).toContain('That is not a pairing link.');
});
