/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
vi.mock('@react-three/fiber', () => ({ Canvas: () => null }));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Ada' }));
vi.mock('./HeroAgentPortrait', () => ({ HeroAgentPortrait: ({ size, label }: { size: string; label: string }) => <div data-portrait-size={size} aria-label={label} /> }));
vi.mock('./ForumPassport', () => ({ ForumPassport: () => <div data-testid="passport-record">Optional identity record</div> }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => <div>Existing tools and skills</div> }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => <div>Existing shared conversation</div> }));
vi.mock('./forumQuery', () => ({ askFromForum: vi.fn() }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: false }) }));
import { ForumView } from './ForumView';
import { askFromForum } from './forumQuery';
import { useCommandCenter } from '../../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
let previous: ReturnType<typeof useCommandCenter.getState>;
const ensureSession = vi.fn(), sendMessage = vi.fn();
beforeEach(() => {
  previous = useCommandCenter.getState();
  vi.clearAllMocks();
  window.history.replaceState({}, '', '/');
  localStorage.clear();
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false }));
  useCommandCenter.setState({ chatSessionId: 'kept-session', chatDockOpen: false, chatWindowOpen: false, voiceEngine: null, worldConversationActive: false, ensureSession, sendMessage });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  act(() => root.render(<ForumView />));
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent === 'Agent desk')!.click());
});
afterEach(() => {
  act(() => root.unmount()); host.remove(); useCommandCenter.setState(previous);
  vi.unstubAllGlobals();
});
const profile = () => host.querySelector('.agent-desk-profile')!;
const clickProfile = (text: string) => act(() => [...profile().querySelectorAll('button')].find(button => button.textContent?.includes(text))!.click());

it('shows a compact named profile and keeps optional identity unmounted until expanded', () => {
  expect(profile().querySelector('h3')?.textContent).toBe('Ada');
  expect(profile().querySelector('[data-portrait-size]')?.getAttribute('data-portrait-size')).toBe('profile');
  expect(profile().querySelector('[role=status]')?.textContent).toContain('Reported:');
  expect(profile().querySelector('.agent-desk-suggested-question')?.textContent).toBe('“Help me turn this idea into a clear plan and identify which agents should contribute.”');
  expect(profile().querySelector('.agent-desk-question')?.textContent).not.toContain('Calm, attentive');
  expect(host.querySelector('[data-testid=passport-record]')).toBeNull();
  const identity = [...profile().querySelectorAll('details')].find(details => details.querySelector('summary')?.textContent?.includes('Identity'))!;
  expect(identity.open).toBe(false);
  act(() => { identity.open = true; identity.dispatchEvent(new Event('toggle')); });
  expect(host.querySelector('[data-testid=passport-record]')).not.toBeNull();
  act(() => { identity.open = false; identity.dispatchEvent(new Event('toggle')); });
  expect(host.querySelector('[data-testid=passport-record]')).toBeNull();
  expect(ensureSession).not.toHaveBeenCalled(); expect(sendMessage).not.toHaveBeenCalled();
});

it('opens the canonical shared chat without sending or replacing the current session', () => {
  clickProfile('Chat');
  expect(useCommandCenter.getState().chatDockOpen).toBe(true);
  expect(useCommandCenter.getState().chatSessionId).toBe('kept-session');
  expect(host.textContent).toContain('Existing shared conversation');
  expect(ensureSession).not.toHaveBeenCalled(); expect(sendMessage).not.toHaveBeenCalled();
  expect(askFromForum).not.toHaveBeenCalled();
});

it('opens a job draft and role questions without dispatching them', () => {
  clickProfile('Write a brief');
  expect((host.querySelector('#forum-kind') as HTMLSelectElement).value).toBe('Job');
  expect((host.querySelector('#forum-brief') as HTMLTextAreaElement).value).toBe('');
  act(() => [...host.querySelectorAll('.forum-tabs button')].find(button => button.textContent === 'Meet')!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  clickProfile('Try a question');
  expect((host.querySelector('#forum-kind') as HTMLSelectElement).value).toBe('Query');
  expect((host.querySelector('#forum-brief') as HTMLTextAreaElement).value).toBe('Help me turn this idea into a clear plan and identify which agents should contribute.');
  expect(askFromForum).not.toHaveBeenCalled(); expect(sendMessage).not.toHaveBeenCalled();
});

it('keeps seating explicit and hides sovereign identity/seating for a specialist', () => {
  clickProfile('Sit together');
  expect(useCommandCenter.getState().worldConversationActive).toBe(true);
  expect(host.querySelector('[aria-label="One on one with Ada"]')).not.toBeNull();
  act(() => (host.querySelector('button[aria-label="Leave table"]') as HTMLButtonElement).click());
  const select = host.querySelector('#forum-agent') as HTMLSelectElement;
  act(() => { select.value = 'librarian'; select.dispatchEvent(new Event('change', { bubbles: true })); });
  expect(profile().textContent).toContain('with this role as context');
  expect(profile().textContent).not.toContain('Sit together');
  expect(profile().textContent).not.toContain('Identity & passport');
  expect(askFromForum).not.toHaveBeenCalled(); expect(sendMessage).not.toHaveBeenCalled();
});

it('keeps work, tool knowledge and world settings available in closed disclosures', () => {
  const disclosures = [...host.querySelectorAll('.agent-desk-support details')];
  expect(disclosures.map(details => details.querySelector('summary')?.textContent)).toEqual(['Work in flight Feed pending', 'Tools & skills', 'World settings']);
  expect(disclosures.every(details => !(details as HTMLDetailsElement).open)).toBe(true);
  expect(host.textContent).toContain('Waiting for the job feed. No activity is inferred.');
  expect(host.textContent).toContain('Existing tools and skills');
  expect(host.querySelector('[aria-label="World lighting"]')).not.toBeNull();
});
