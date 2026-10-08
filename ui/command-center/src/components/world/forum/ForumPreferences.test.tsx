/** @vitest-environment jsdom */
import { act, type ReactElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const renderer = vi.hoisted(() => ({ fail: false }));
vi.mock('@react-three/fiber', () => ({
  Canvas: ({ children }: { children: ReactElement<{ reducedMotion: boolean }> }) => {
    if (renderer.fail) throw new Error('Hero asset unavailable');
    return <div data-portrait-reduced={String(children.props.reducedMotion)} />;
  },
}));
import { HeroAgentPortrait } from './HeroAgentPortrait';
import { useHudPresence, HUD_REST_DELAY_MS } from './useHudPresence';
import { useForumLighting, FORUM_LIGHTING_KEY } from './useForumLighting';
import { setReduceMotion } from '../../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement, systemReduced = false;
const listeners = new Set<() => void>();
function MotionProbe() {
  const hud = useHudPresence({ walking: false });
  return <><span data-hud-awake={String(hud.awake)} /><HeroAgentPortrait id="henry" size="profile" label="Agent portrait" /></>;
}
function LightingProbe() {
  const [lighting, choose] = useForumLighting();
  return <><span data-lighting={lighting} /><button onClick={() => choose('night')}>Night</button><button onClick={() => choose('system')}>System</button></>;
}
beforeEach(() => {
  localStorage.clear(); systemReduced = false; renderer.fail = false; listeners.clear();
  vi.useFakeTimers();
  vi.stubGlobal('matchMedia', vi.fn(() => ({ get matches() { return systemReduced; }, addEventListener: (_type: string, listener: () => void) => listeners.add(listener), removeEventListener: (_type: string, listener: () => void) => listeners.delete(listener) })));
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.clear(); });

it('updates a mounted portrait and HUD when the explicit motion preference changes', () => {
  act(() => root.render(<MotionProbe />));
  expect(host.querySelector('[data-portrait-reduced]')?.getAttribute('data-portrait-reduced')).toBe('false');
  act(() => setReduceMotion(true));
  act(() => vi.advanceTimersByTime(HUD_REST_DELAY_MS + 1));
  expect(host.querySelector('[data-hud-awake]')?.getAttribute('data-hud-awake')).toBe('true');
  expect(host.querySelector('[data-portrait-reduced]')?.getAttribute('data-portrait-reduced')).toBe('true');
  act(() => setReduceMotion(false));
  act(() => vi.advanceTimersByTime(HUD_REST_DELAY_MS + 1));
  expect(host.querySelector('[data-hud-awake]')?.getAttribute('data-hud-awake')).toBe('false');
  expect(host.querySelector('[data-portrait-reduced]')?.getAttribute('data-portrait-reduced')).toBe('false');
  act(() => setReduceMotion(true));
  expect(host.querySelector('[data-hud-awake]')?.getAttribute('data-hud-awake')).toBe('true');
});

it('responds to live OS changes while preserving explicit override precedence and cleaning listeners', () => {
  const schedule = vi.spyOn(globalThis, 'setTimeout');
  const cancel = vi.spyOn(globalThis, 'clearTimeout');
  act(() => root.render(<MotionProbe />));
  act(() => { systemReduced = true; listeners.forEach(listener => listener()); });
  expect(host.querySelector('[data-portrait-reduced]')?.getAttribute('data-portrait-reduced')).toBe('true');
  act(() => setReduceMotion(false));
  expect(host.querySelector('[data-portrait-reduced]')?.getAttribute('data-portrait-reduced')).toBe('false');
  const hudTimerIndex = schedule.mock.calls.map(call => call[1]).lastIndexOf(HUD_REST_DELAY_MS);
  expect(hudTimerIndex).toBeGreaterThanOrEqual(0);
  const hudTimer = schedule.mock.results[hudTimerIndex].value;
  act(() => root.render(null));
  expect(listeners.size).toBe(0);
  expect(cancel).toHaveBeenCalledWith(hudTimer);
});

it('restores lighting after a remount and keeps System as a persisted choice', () => {
  act(() => root.render(<LightingProbe />));
  act(() => host.querySelector('button')!.click());
  expect(localStorage.getItem(FORUM_LIGHTING_KEY)).toBe('night');
  act(() => root.render(null)); act(() => root.render(<LightingProbe />));
  expect(host.querySelector('[data-lighting]')?.getAttribute('data-lighting')).toBe('night');
  act(() => [...host.querySelectorAll('button')].find(button => button.textContent === 'System')!.click());
  expect(localStorage.getItem(FORUM_LIGHTING_KEY)).toBe('system');
  expect([...Array(localStorage.length)].map((_, index) => localStorage.key(index))).toEqual([FORUM_LIGHTING_KEY]);
});

it('rejects corrupt lighting values and remains usable when storage is unavailable', () => {
  localStorage.setItem(FORUM_LIGHTING_KEY, 'invalid');
  act(() => root.render(<LightingProbe />));
  expect(host.querySelector('[data-lighting]')?.getAttribute('data-lighting')).toBe('system');
  act(() => root.render(null));
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('Storage blocked'); });
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('Storage blocked'); });
  act(() => root.render(<LightingProbe />));
  act(() => host.querySelector('button')!.click());
  expect(host.querySelector('[data-lighting]')?.getAttribute('data-lighting')).toBe('night');
});

it('contains a failed portrait render and leaves adjacent profile controls usable', () => {
  renderer.fail = true;
  vi.spyOn(console, 'error').mockImplementation(() => {});
  const click = vi.fn();
  act(() => root.render(<><HeroAgentPortrait id="henry" size="profile" label="Agent portrait" /><button onClick={click}>Chat</button></>));
  expect(host.textContent).toContain('Portrait unavailable');
  expect(host.querySelector('[data-testid="live-agent-portrait"]')?.getAttribute('aria-label')).toBe('Portrait unavailable');
  act(() => host.querySelector('button')!.click());
  expect(click).toHaveBeenCalledOnce();
});
