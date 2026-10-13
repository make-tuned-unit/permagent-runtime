import { sceneChildren, settleStartup } from './forumStartupTestUtils';
/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const gpu = vi.hoisted(() => ({ props: null as Record<string, any> | null }));
vi.mock('@react-three/fiber', () => ({ Canvas: (props: Record<string, any>) => { if (props.className === 'forum-canvas') gpu.props = props; return null; } }));
vi.mock('./HeroAgentPortrait', () => ({ HeroAgentPortrait: () => null }));
vi.mock('./ForumPassport', () => ({ ForumPassport: () => null }));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Aster' }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: true }) }));
import { ForumView } from './ForumView';
import { HabitatNavigation } from './HabitatScene';
import { ForumLightingPreparation } from './ForumLightingPreparation';
import { ForumEnvironment } from './ForumEnvironment';
import { ForumStageReady } from './ForumStageStatus';
import { HABITAT_ARRIVAL } from './habitatLayout';
import { ForumWalk } from './ForumWalk';
import { setTheme } from '../../../styles/tokens';
import { useCommandCenter } from '../../../lib/store';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement;
let shellWidth = 1024;
class Observer { constructor(private callback: ResizeObserverCallback) {} observe() { this.callback([{ contentRect: { width: shellWidth, height: 640 } }] as ResizeObserverEntry[], this as unknown as ResizeObserver); } disconnect() {} }
beforeEach(() => {
  localStorage.clear(); localStorage.setItem('permagent.forum.lighting', 'day');
  shellWidth = 1024;
  localStorage.setItem('permagent-reduce-motion', 'true');
  window.history.replaceState({}, '', '/'); vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false })));
  vi.stubGlobal('ResizeObserver', Observer);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => ({ width: shellWidth, height: 640, top: 0, bottom: 640, left: 0, right: shellWidth, x: 0, y: 0, toJSON: () => ({}) }));
  useCommandCenter.setState({ daemonAuthError: null, chatDockOpen: false, chatLauncherSize: { width: 44, height: 44 } });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); act(() => setTheme('dark')); });
async function render() {
  await act(async () => root.render(<ForumView />));
  await settleStartup();
  act(() => {
    const children = sceneChildren(gpu.props!);
    children.find((c: any) => c?.type === ForumStageReady).props.onReady();
    children.find((c: any) => c?.type === ForumLightingPreparation).props.onPrepared('day');
  });
  act(() => sceneChildren(gpu.props!).find((c: any) => c?.type === 'group').props.children.find((c: any) => c?.type === ForumEnvironment).props.onApplied('day'));
}
function button(text: string) { return [...host.querySelectorAll('button')].find(b => b.textContent === text)!; }
function click(text: string) { act(() => button(text).click()); }
it('keeps the same walk mounted with input suspended across workspace leave and return', async () => {
  await render(); click('Walk the world');
  const walk = () => sceneChildren(gpu.props!).find((child: any) => child?.type === ForumWalk);
  const start = walk().props.start;
  expect(walk().props.active).toBe(true);
  act(() => root.render(<ForumView visible={false} />));
  expect(walk().props.active).toBe(false); expect(walk().props.start).toBe(start);
  expect(sceneChildren(gpu.props!).some((child: any) => child?.type === HabitatNavigation)).toBe(false);
  act(() => root.render(<ForumView />));
  expect(walk().props.active).toBe(true); expect(walk().props.start).toBe(start);
  expect(button('Walk the world')).toBeUndefined();
  expect(button('Overview').getAttribute('aria-pressed')).toBe('false');
});
it('arrives at eye level with the desk closed and exposes exactly one explicit overview action while walking', async () => {
  await render(); expect(gpu.props!.camera.position).toEqual(HABITAT_ARRIVAL.position);
  expect(gpu.props!.camera).toMatchObject({ fov: 72, near: .25, far: 6000 });
  expect(host.querySelector('.forum-shell')?.getAttribute('data-desk')).toBe('closed');
  expect(localStorage.getItem('permagent.forum.desk')).toBeNull();
  expect(button('Overview').getAttribute('aria-pressed')).toBe('false');
  expect(sceneChildren(gpu.props!).find((c: any) => c?.type === HabitatNavigation).props.overview).toBe(false);
  click('Walk the world'); expect(button('Walk the world')).toBeUndefined();
  expect([...host.querySelectorAll('button')].filter(b => /overview/i.test(b.textContent ?? ''))).toHaveLength(1);
  click('Overview'); expect(button('Overview').getAttribute('aria-pressed')).toBe('true');
  expect(sceneChildren(gpu.props!).find((c: any) => c?.type === HabitatNavigation).props.overview).toBe(true);
});
it('preserves the chosen place, desk and explicit overview across workspace leave and return', async () => {
  await render(); click('Agent desk');
  const archive = [...host.querySelectorAll<HTMLButtonElement>('[role="radio"]')].find(b => b.textContent?.includes('Garden archive'))!;
  act(() => archive.click());
  const navigation = () => sceneChildren(gpu.props!).find((c: any) => c?.type === HabitatNavigation).props;
  expect(navigation().district.id).toBe('gallery');
  act(() => root.render(<ForumView visible={false} />));
  act(() => root.render(<ForumView />));
  expect(navigation().district.id).toBe('gallery'); expect(navigation().overview).toBe(false);
  expect(host.querySelector('.forum-shell')?.getAttribute('data-desk')).toBe('open');
  click('Overview'); const revision = navigation().overviewRevision;
  act(() => root.render(<ForumView visible={false} />)); act(() => root.render(<ForumView />));
  expect(navigation().overview).toBe(true); expect(navigation().overviewRevision).toBe(revision);
  click('Overview'); expect(navigation().overviewRevision).toBe(revision + 1);
});
it.each(['open', 'closed', 'invalid', null])('only restores an explicit desk preference at launch: %s', async preference => {
  if (preference) localStorage.setItem('permagent.forum.desk', preference);
  await render();
  expect(host.querySelector('.forum-shell')?.getAttribute('data-desk')).toBe(preference === 'open' ? 'open' : 'closed');
});
it('keeps automatic desk opening in-session without making it a launch default', async () => {
  await render(); click('Give a brief');
  expect(host.querySelector('.forum-shell')?.getAttribute('data-desk')).toBe('open');
  act(() => root.render(<ForumView visible={false} />)); act(() => root.render(<ForumView />));
  expect(host.querySelector('.forum-shell')?.getAttribute('data-desk')).toBe('open');
  expect(localStorage.getItem('permagent.forum.desk')).toBeNull();
  act(() => root.unmount()); root = createRoot(host); await render();
  expect(host.querySelector('.forum-shell')?.getAttribute('data-desk')).toBe('closed');
});
it('persists only the deliberate desk toggle and starts a new camera session at arrival', async () => {
  await render(); click('Agent desk'); click('Overview');
  expect(localStorage.getItem('permagent.forum.desk')).toBe('open');
  act(() => root.unmount()); root = createRoot(host); await render();
  expect(host.querySelector('.forum-shell')?.getAttribute('data-desk')).toBe('open');
  expect(button('Overview').getAttribute('aria-pressed')).toBe('false');
  click('Agent desk'); expect(localStorage.getItem('permagent.forum.desk')).toBe('closed');
});
it('opens lighting directly from the HUD and Escape closes only appearance, restoring focus', async () => {
  await render(); click('Agent desk'); click('World appearance');
  const select = host.querySelector<HTMLSelectElement>('[aria-label="World lighting"]')!;
  expect(select.closest('[role="dialog"]')?.hasAttribute('hidden')).toBe(false);
  expect(document.activeElement).toBe(select);
  act(() => select.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  expect(select.closest('[role="dialog"]')?.hasAttribute('hidden')).toBe(true);
  expect(document.activeElement).toBe(button('World appearance'));
  expect(host.querySelector('.forum-shell')?.getAttribute('data-desk')).toBe('open');
  expect(host.querySelector('.agent-desk-body')?.textContent).toContain('World details & attribution');
});
it('keeps Close outside the scrolling desk body and preserves its focus return', async () => {
  await render(); click('Agent desk');
  const body = host.querySelector<HTMLElement>('.agent-desk-body')!;
  const close = host.querySelector<HTMLButtonElement>('[aria-label="Close agent desk"]')!;
  expect(body.contains(close)).toBe(false); expect(body.style.overflowY).toBe('auto');
  expect(close.parentElement!.style.position).toBe('sticky');
  body.scrollTop = 1000; act(() => close.click());
  expect(document.activeElement).toBe(button('Agent desk'));
  expect(host.querySelector('.forum-shell')?.getAttribute('data-desk')).toBe('closed');
});
for (const [width, height] of [[1280, 800], [1024, 640]]) for (const theme of ['dark', 'silver'] as const) for (const dock of [false, true]) for (const screen of ['arrival', 'overview', 'appearance', 'desk', 'overview-desk'] as const) {
  it(`DOM World ${screen} ${width}x${height} ${theme} dock-${dock ? 'open' : 'closed'}`, async () => {
    Object.defineProperties(window, { innerWidth: { configurable: true, value: width }, innerHeight: { configurable: true, value: height } });
    shellWidth = width - 240 - (dock ? (width < 1200 ? 320 : 384) : 0);
    host.style.width = `${shellWidth}px`; host.style.height = `${height - 28}px`;
    act(() => { setTheme(theme); useCommandCenter.setState({ chatDockOpen: dock, chatLauncherSize: dock ? null : { width: 44, height: 44 } }); });
    await render(); if (screen.startsWith('overview')) click('Overview'); if (screen.includes('desk')) click('Agent desk'); if (screen === 'appearance') click('World appearance');
    expect(host).toMatchSnapshot();
  });
}
