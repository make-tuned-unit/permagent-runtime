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
import { HABITAT_OVERVIEW } from './habitatLayout';
import { setTheme } from '../../../styles/tokens';
import { useCommandCenter } from '../../../lib/store';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement;
let shellWidth = 1024;
class Observer { constructor(private callback: ResizeObserverCallback) {} observe() { this.callback([{ contentRect: { width: shellWidth, height: 640 } }] as ResizeObserverEntry[], this as unknown as ResizeObserver); } disconnect() {} }
beforeEach(() => {
  localStorage.clear(); localStorage.setItem('permagent.forum.lighting', 'day'); localStorage.setItem('permagent.forum.desk', 'closed');
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
  act(() => {
    const children = gpu.props!.children;
    children.find((c: any) => c?.type === ForumStageReady).props.onReady();
    children.find((c: any) => c?.type === ForumLightingPreparation).props.onPrepared('day');
  });
  act(() => gpu.props!.children.find((c: any) => c?.type === 'group').props.children.find((c: any) => c?.type === ForumEnvironment).props.onApplied('day'));
}
function button(text: string) { return [...host.querySelectorAll('button')].find(b => b.textContent === text)!; }
function click(text: string) { act(() => button(text).click()); }
it('arrives in the frozen overview and exposes exactly one overview action while walking', async () => {
  await render(); expect(gpu.props!.camera.position).toEqual(HABITAT_OVERVIEW.position);
  expect(button('Overview').getAttribute('aria-pressed')).toBe('true');
  click('Walk the world'); expect(button('Walk the world')).toBeUndefined();
  expect([...host.querySelectorAll('button')].filter(b => /overview/i.test(b.textContent ?? ''))).toHaveLength(1);
  click('Overview'); expect(button('Overview').getAttribute('aria-pressed')).toBe('true');
  expect(gpu.props!.children.find((c: any) => c?.type === HabitatNavigation).props.overview).toBe(true);
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
for (const [width, height] of [[1280, 800], [1024, 640]]) for (const theme of ['dark', 'silver'] as const) for (const dock of [false, true]) for (const screen of ['overview', 'appearance', 'desk'] as const) {
  it(`DOM World ${screen} ${width}x${height} ${theme} dock-${dock ? 'open' : 'closed'}`, async () => {
    Object.defineProperties(window, { innerWidth: { configurable: true, value: width }, innerHeight: { configurable: true, value: height } });
    shellWidth = width - 240 - (dock ? (width < 1200 ? 320 : 384) : 0);
    host.style.width = `${shellWidth}px`; host.style.height = `${height - 28}px`;
    act(() => { setTheme(theme); useCommandCenter.setState({ chatDockOpen: dock, chatLauncherSize: dock ? null : { width: 44, height: 44 } }); });
    await render(); if (screen === 'desk') click('Agent desk'); if (screen === 'appearance') click('World appearance');
    expect(host).toMatchSnapshot();
  });
}
