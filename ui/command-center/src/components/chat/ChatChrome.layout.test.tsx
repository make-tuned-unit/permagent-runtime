/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ChatLauncher, footerClearance, LAUNCHER_FOOTER_ATTR } from './ChatLauncher';
import { ChatDock } from './ChatDock';
import { Browser } from '../browser/Browser';
import { useCommandCenter } from '../../lib/store';
import { setTheme } from '../../styles/tokens';

vi.mock('../../lib/api', () => ({ api: {
  getIdentity: async () => ({ first_name: 'Agent' }),
  getBrowserHistory: async () => ({ entries: [] }),
  getBrowserBookmarks: async () => ({ bookmarks: [] }),
  getBrowserTabSets: async () => ({ tabSets: [] }),
  getPacks: async () => ({ packs: [] }),
}, apiFetch: async () => [] }));
vi.mock('../voice/VoiceButton', () => ({ VoiceButton: () => null }));
vi.mock('../voice/VoiceOrb', () => ({ VoiceOrb: () => null }));
vi.mock('./SkillPromptBanner', () => ({ SkillPromptBanner: () => null }));
vi.mock('./RoleRoutingPrompt', () => ({ RoleRoutingPrompt: () => null }));
vi.mock('../../hooks/useBrowserContentBridge', () => ({ useBrowserContentBridge: () => {} }));
vi.mock('../../hooks/useBrowserActBridge', () => ({ useBrowserActBridge: () => {} }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement; let root: Root;
let footerTop: number;
const original = useCommandCenter.getState();
class Observer { observe() {} disconnect() {} unobserve() {} }
function viewport(width: number, height: number) {
  Object.defineProperties(window, { innerWidth: { configurable: true, value: width }, innerHeight: { configurable: true, value: height } });
  footerTop = height - 28;
  host.style.cssText = `width:${width}px;height:${height}px;display:flex`;
}
beforeEach(() => {
  localStorage.clear(); localStorage.setItem('permagent-reduce-motion', 'true');
  vi.stubGlobal('ResizeObserver', Observer);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const footer = this.hasAttribute(LAUNCHER_FOOTER_ATTR);
    const launcher = this.hasAttribute('data-chat-launcher');
    return { x: 0, y: footer ? footerTop : 0, top: footer ? footerTop : 0, left: 0,
      width: footer ? 500 : launcher ? 44 : 0, height: footer ? 28 : launcher ? 44 : 0,
      right: 500, bottom: window.innerHeight, toJSON: () => ({}) };
  });
  useCommandCenter.setState({ chatDockOpen: false, chatWindowOpen: false, chatLauncherSize: null,
    providers: [], providersError: false, currentModel: null, chatResolvedRoute: { provider: '', model: '', source: 'unconfigured' },
    chatMessages: [], chatHistoryLoaded: true, chatDraft: { text: 'Keep my unsent draft', files: [new File(['note'], 'notes.txt')] },
    loadProviders: vi.fn(async () => {}), ensureSession: vi.fn(async () => null), disconnectSession: vi.fn(),
  });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host); viewport(1024, 640);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); useCommandCenter.setState(original); act(() => setTheme('dark')); });
async function render() {
  await act(async () => root.render(<><main style={{ flex: 1, minWidth: 0 }}><Browser detached initialTab={{ id: 'fixture', label: 'New Tab', url: '', loading: false, webviewId: null }} /></main><ChatLauncher /><ChatDock /></>));
}
it('raises the launcher above Browser and publishes the same raised native clearance', async () => {
  await render();
  expect(host.querySelector(`[${LAUNCHER_FOOTER_ATTR}]`)).toBeTruthy();
  expect(footerClearance()).toBe(28);
  expect(host.querySelector<HTMLElement>('[data-chat-launcher]')!.style.bottom).toBe('48px');
  expect(useCommandCenter.getState().chatLauncherSize).toEqual({ width: 44, height: 44, bottomInset: 28 });
  footerTop = 400;
  expect(footerClearance()).toBe(240); // A floating recorder includes the gap below it.
});
it('closes compact Chat immediately from the keyboard, restores the launcher and preserves text and files', async () => {
  useCommandCenter.setState({ chatDockOpen: true }); await render();
  const dock = host.querySelector<HTMLElement>('[data-testid="chat-dock"]')!;
  expect(dock.style.width).toBe('320px');
  expect(host.querySelector('[role="separator"]')).toBeNull();
  const close = host.querySelector<HTMLButtonElement>('[aria-label="Close chat"]')!;
  expect(close.textContent).toContain('Close');
  close.focus(); act(() => close.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
  expect(host.querySelector('[data-testid="chat-dock"]')).toBeNull();
  expect(useCommandCenter.getState().chatDraft.text).toBe('Keep my unsent draft');
  expect(useCommandCenter.getState().chatDraft.files[0].name).toBe('notes.txt');
  await act(async () => { await new Promise(resolve => requestAnimationFrame(resolve)); });
  expect(document.activeElement).toBe(host.querySelector('[data-chat-launcher]'));
  await act(async () => useCommandCenter.getState().openChatDock());
  expect(host.querySelector('textarea')?.value).toBe('Keep my unsent draft');
  expect(host.textContent).toContain('notes.txt');
});
it('does not dismiss Chat through a higher dialog', async () => {
  useCommandCenter.setState({ chatDockOpen: true }); await render();
  const dialog = document.createElement('div'); dialog.setAttribute('aria-modal', 'true'); document.body.append(dialog);
  act(() => host.querySelector('[aria-label="Close chat"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  expect(useCommandCenter.getState().chatDockOpen).toBe(true); dialog.remove();
});
for (const [width, height] of [[1280, 800], [1024, 640]]) for (const theme of ['dark', 'silver'] as const) for (const open of [false, true]) {
  it(`DOM Browser and Chat ${width}x${height} ${theme} dock-${open ? 'open' : 'closed'}`, async () => {
    viewport(width, height); act(() => { setTheme(theme); useCommandCenter.setState({ chatDockOpen: open }); });
    await render(); expect(host).toMatchSnapshot();
  });
}
