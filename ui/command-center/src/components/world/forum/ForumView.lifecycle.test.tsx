/** @vitest-environment jsdom */
// Actual World HUD, portrait idle scheduler, walk/fly controllers and visibility
// effects. GPU/context allocations are counted proxies, not native VRAM evidence.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const gpu = vi.hoisted(() => ({ created: 0, live: 0, invalidations: 0,
  loops: new Map<HTMLCanvasElement, string>(), frames: new Map<HTMLCanvasElement, Set<() => void>>() }));
vi.mock('@react-three/fiber', async () => {
  const React = await import('react'); const { PerspectiveCamera } = await import('three');
  const Context = React.createContext<any>(null);
  return {
    Canvas: ({ children, frameloop, className }: any) => {
      const host = React.useRef<HTMLDivElement>(null);
      const state = React.useMemo(() => ({ camera: new PerspectiveCamera(), gl: { domElement: document.createElement('canvas') }, invalidate: () => gpu.invalidations++, setEvents: () => {} }), []);
      const canvas = state.gl.domElement;
      React.useLayoutEffect(() => { host.current!.append(canvas); gpu.created++; gpu.live++; gpu.frames.set(canvas, new Set()); return () => { gpu.live--; gpu.loops.delete(canvas); gpu.frames.delete(canvas); }; }, [canvas]);
      gpu.loops.set(canvas, frameloop);
      const pick = (nodes: any): any => React.Children.map(nodes, (child: any) => {
        if (!React.isValidElement<any>(child)) return null;
        if (typeof child.type === 'function' && ['ForumWalk', 'ForumFlight', 'ForumStageReady', 'ForumCanvasActivity'].includes(child.type.name)) return child;
        return pick(child.props.children);
      });
      return <Context.Provider value={state}><div ref={host} data-canvas={className ?? 'portrait'}>{className === 'forum-canvas' ? pick(children) : children}</div></Context.Provider>;
    },
    useThree: (selector?: (value: any) => any) => { const value = React.useContext(Context); return selector ? selector(value) : value; },
    useFrame: (callback: (...args: any[]) => void) => { const value = React.useContext(Context); React.useEffect(() => { const set = gpu.frames.get(value.gl.domElement); const frame = () => callback({ ...value, clock: { elapsedTime: 0 } }, 1 / 60); set?.add(frame); return () => { set?.delete(frame); }; }, [value, callback]); },
  };
});
vi.mock('@react-three/drei', async original => { const { Group } = await import('three'); const scene = new Group(); return { ...await original<any>(), useGLTF: Object.assign(() => ({ scene }), { preload: () => {}, clear: () => {} }) }; });
vi.mock('./heroRobot', async original => { const { Group } = await import('three'); return { ...await original<any>(), buildHeroRobot: () => ({ object: new Group(), head: null }), disposeHeroRobot: () => {} }; });
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: true }) }));
import { ForumView } from './ForumView';
import { setReduceMotion } from '../../../styles/tokens';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement, hidden = false;
const listeners = new Map<EventTarget, Map<string, Set<EventListenerOrEventListenerObject>>>();
const tracked = new Set(['keydown', 'keyup', 'mousemove', 'mouseup', 'mousedown', 'blur', 'focusin', 'visibilitychange', 'pointerlockchange', 'webglcontextlost']);
function countListeners(inputOnly = false) { let count = 0; for (const [target, byType] of listeners) for (const [type, set] of byType) if (!inputOnly || ((target === window || target === document) && ['keydown', 'keyup', 'mousemove', 'mouseup'].includes(type))) count += set.size; return count; }
function snapshot() { return { canvases: host.querySelectorAll('canvas').length, contexts: gpu.live, listeners: countListeners(), inputListeners: countListeners(true), timers: vi.getTimerCount() }; }
function tick() { act(() => { for (const [canvas, callbacks] of gpu.frames) if (gpu.loops.get(canvas) !== 'never') callbacks.forEach(callback => callback()); }); }
function show(visible: boolean) { act(() => root.render(<ForumView visible={visible} />)); tick(); }
function hidePage(value: boolean) { hidden = value; act(() => document.dispatchEvent(new Event('visibilitychange'))); tick(); }
function click(text: string) { const button = [...host.querySelectorAll('button')].find(b => b.textContent?.includes(text)); expect(button).toBeTruthy(); act(() => button!.click()); }
beforeEach(() => {
  localStorage.clear(); setReduceMotion(false); vi.useFakeTimers(); hidden = false;
  vi.spyOn(document, 'hidden', 'get').mockImplementation(() => hidden);
  vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false })));
  vi.spyOn(console, 'error').mockImplementation(() => {}); // Three tags in the DOM GPU double.
  gpu.created = 0; gpu.live = 0; gpu.invalidations = 0; gpu.loops.clear(); gpu.frames.clear(); listeners.clear();
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  const add = EventTarget.prototype.addEventListener, remove = EventTarget.prototype.removeEventListener;
  vi.spyOn(EventTarget.prototype, 'addEventListener').mockImplementation(function(this: EventTarget, type, listener, options) {
    if (listener && tracked.has(type)) { if (!listeners.has(this)) listeners.set(this, new Map()); const byType = listeners.get(this)!; if (!byType.has(type)) byType.set(type, new Set()); byType.get(type)!.add(listener); }
    return add.call(this, type, listener, options);
  });
  vi.spyOn(EventTarget.prototype, 'removeEventListener').mockImplementation(function(this: EventTarget, type, listener, options) { if (listener) listeners.get(this)?.get(type)?.delete(listener); return remove.call(this, type, listener, options); });
});
afterEach(() => { act(() => root.unmount()); host.remove(); window.history.replaceState({}, '', '/'); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
it('counts canvases contexts listeners and timers over ten leave-return and hide-resume cycles', () => {
  show(true); const initial = snapshot(); let hiddenInvalidations = 0;
  const cycles = [];
  for (let i = 0; i < 10; i++) {
    show(false);
    expect([...gpu.loops.values()].every(loop => loop === 'never')).toBe(true);
    expect(countListeners(true)).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
    const before = gpu.invalidations;
    act(() => vi.advanceTimersByTime(1000)); hiddenInvalidations += gpu.invalidations - before;
    show(true); hidePage(true); const hiddenCounts = snapshot();
    expect([...gpu.loops.values()].every(loop => loop === 'never')).toBe(true);
    expect(hiddenCounts.timers).toBe(0); hidePage(false);
    cycles.push({ hidden: hiddenCounts, resumed: snapshot() });
    expect(snapshot()).toEqual(initial);
  }
  console.info('L2 lifecycle trace', JSON.stringify({ initial, created: gpu.created, hiddenInvalidations, cycles }));
  expect(hiddenInvalidations).toBe(0);
  expect(gpu.created).toBe(initial.contexts);
  act(() => root.render(null));
  expect(gpu.live).toBe(0);
  expect(countListeners()).toBe(0);
  const idleBefore = gpu.invalidations;
  act(() => vi.advanceTimersByTime(1000));
  expect(gpu.invalidations).toBe(idleBefore);
  expect(vi.getTimerCount()).toBe(0);
});
it('releases flying and walking keys on workspace leave and page hide', () => {
  window.history.replaceState({}, '', '/?campus=1');
  show(true); click('Fly the skiff'); show(false);
  const flight = new KeyboardEvent('keydown', { code: 'ArrowUp', bubbles: true, cancelable: true });
  act(() => document.body.dispatchEvent(flight));
  expect(flight.defaultPrevented).toBe(false);
  expect(countListeners(true)).toBe(0);
  console.info('L2 hidden flight prevented', flight.defaultPrevented);
  show(true); click('Walk'); hidePage(true);
  const walk = new KeyboardEvent('keydown', { code: 'ArrowUp', bubbles: true, cancelable: true });
  act(() => document.body.dispatchEvent(walk));
  expect(walk.defaultPrevented).toBe(false);
  expect(countListeners(true)).toBe(0);
  const escape = new KeyboardEvent('keydown', { key: 'Escape', code: 'Escape', bubbles: true, cancelable: true });
  act(() => document.body.dispatchEvent(escape)); expect(escape.defaultPrevented).toBe(false);
  console.info('L2 hidden walk prevented', walk.defaultPrevented);
});

it('defers every canvas until first visibility and cancels idle timers for reduced motion', () => {
  show(false); expect(gpu.created).toBe(0);
  show(true); expect(gpu.live).toBeGreaterThan(0);
  act(() => setReduceMotion(true));
  const idleBefore = gpu.invalidations;
  act(() => vi.advanceTimersByTime(1000));
  expect(gpu.invalidations).toBe(idleBefore);
  expect(vi.getTimerCount()).toBe(0);
});

it('keeps context-loss recovery visible until an explicit retry', () => {
  show(true);
  const canvas = host.querySelector('[data-canvas="forum-canvas"] canvas')!;
  const lost = new Event('webglcontextlost', { cancelable: true });
  act(() => canvas.dispatchEvent(lost));
  expect(lost.defaultPrevented).toBe(true);
  tick();
  expect(host.textContent).toContain('World could not start.');
  click('Retry World'); tick();
  expect(host.textContent).not.toContain('World could not start.');
});
