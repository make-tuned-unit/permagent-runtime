import { sceneChildren, settleStartup } from './forumStartupTestUtils';
/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const gpu = vi.hoisted(() => ({ props: null as Record<string, any> | null }));
vi.mock('@react-three/fiber', () => ({ Canvas: (props: Record<string, any>) => { gpu.props = props; return null; } }));
vi.mock('./HeroAgentPortrait', () => ({ HeroAgentPortrait: () => null }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: true }) }));
import { ForumView } from './ForumView';
import { ForumLightingPreparation } from './ForumLightingPreparation';
import { ForumEnvironment } from './ForumEnvironment';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
beforeEach(async () => { localStorage.clear(); localStorage.setItem('permagent.forum.lighting', 'day'); vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false }))); host = document.createElement('div'); document.body.append(host); root = createRoot(host); act(() => root.render(<ForumView />)); await settleStartup(); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
function preparation() { return sceneChildren(gpu.props!).find((child: any) => child?.type === ForumLightingPreparation).props; }
function environment() { return sceneChildren(gpu.props!).find((child: any) => child?.type === 'group').props.children.find((child: any) => child?.type === ForumEnvironment).props; }
function select(value: string) { act(() => { const control = host.querySelector('[aria-label="World lighting"]') as HTMLSelectElement; control.value = value; control.dispatchEvent(new Event('change', { bubbles: true })); }); }
function status() { return host.querySelector('[data-testid="forum-lighting-status"]')!.textContent; }
it('acknowledges selection immediately and retains applied day until night is ready', () => {
  act(() => { preparation().onPrepared('day'); environment().onApplied('day'); });
  const start = performance.now(); select('night');
  expect((host.querySelector('select[aria-label="World lighting"]') as HTMLSelectElement).value).toBe('night');
  expect(status()).toContain('Preparing night'); expect(status()).toContain('Day is still applied');
  expect(environment().appearance).toBe('day');
  const acknowledgedMs = performance.now() - start;
  act(() => preparation().onPrepared('night'));
  expect(environment().appearance).toBe('night'); expect(status()).toContain('Preparing night');
  act(() => environment().onApplied('night'));
  expect(status()).toBe('Night lighting applied.');
  expect(acknowledgedMs).toBeLessThan(200);
  console.info('L2 lighting acknowledgment ms', acknowledgedMs);
});
it('latest day wins after rapid day night day and late night callbacks', () => {
  act(() => { preparation().onPrepared('day'); environment().onApplied('day'); }); select('night');
  const late = preparation(); select('day');
  act(() => { late.onPrepared('night'); environment().onApplied('night'); late.onFailure('night'); });
  expect(environment().appearance).toBe('day');
  act(() => { preparation().onPrepared('day'); environment().onApplied('day'); });
  expect(status()).toBe('Day lighting applied.');
});
it('reports failed preparation without removing the usable lighting', () => {
  act(() => { preparation().onPrepared('day'); environment().onApplied('day'); }); select('night');
  act(() => preparation().onFailure('night'));
  expect(status()).toContain('Could not apply night'); expect(environment().appearance).toBe('day');
  expect([...host.querySelectorAll('button')].some(b => b.textContent === 'Retry lighting')).toBe(true);
});
