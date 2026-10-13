/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { PerspectiveCamera, Vector3 } from 'three';
import { OrbitControls as OrbitControlsImpl } from 'three-stdlib';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { settleStartup } from './forumStartupTestUtils';

// Exercise the actual HUD command, navigation effects and Three controls. Only
// the GPU renderer/scene decoration is omitted; frame deltas and measured canvas
// dimensions are supplied explicitly (jsdom does not lay out the app shell).
const scene = vi.hoisted(() => ({
  camera: null as any, controls: null as any, gl: {} as any, setEvents: () => {},
  size: { width: 1024, height: 640 }, frame: null as any, props: null as any,
}));
vi.mock('@react-three/fiber', async () => {
  const React = await import('react');
  return {
    Canvas: ({ children }: any) => <>{React.Children.toArray(children).filter((child: any) =>
      ['HabitatNavigation', 'ForumCanvasActivity'].includes(child?.type?.name))}</>,
    useThree: (pick: (s: typeof scene) => unknown) => pick(scene),
    useFrame: (frame: unknown) => { scene.frame = frame; },
  };
});
vi.mock('@react-three/drei', async original => {
  const React = await import('react');
  return { ...await original<any>(), OrbitControls: React.forwardRef((props: any, ref) => {
    if (scene.props?.enabled !== props.enabled) scene.controls.enabled = props.enabled;
    scene.controls.minDistance = props.minDistance;
    scene.controls.maxDistance = props.maxDistance;
    scene.controls.maxPolarAngle = props.maxPolarAngle;
    scene.props = props;
    React.useImperativeHandle(ref, () => scene.controls);
    React.useEffect(() => {
      scene.controls.addEventListener('start', props.onStart);
      return () => scene.controls.removeEventListener('start', props.onStart);
    }, [props.onStart]);
    return null;
  }) };
});
vi.mock('./HeroAgentPortrait', () => ({ HeroAgentPortrait: () => null }));
vi.mock('./ForumPassport', () => ({ ForumPassport: () => null }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Aster' }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: true }) }));
import { ForumView } from './ForumView';
import { HABITAT_ARRIVAL } from './habitatLayout';
import { setReduceMotion } from '../../../styles/tokens';
import { useCommandCenter } from '../../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
class Observer {
  constructor(private callback: ResizeObserverCallback) {}
  observe() { this.callback([{ contentRect: scene.size }] as ResizeObserverEntry[], this as unknown as ResizeObserver); }
  disconnect() {}
}
beforeEach(() => {
  localStorage.clear(); setReduceMotion(false);
  localStorage.setItem('permagent.forum.lighting', 'day');
  window.history.replaceState({}, '', '/');
  vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false })));
  vi.stubGlobal('ResizeObserver', Observer);
  vi.stubGlobal('innerWidth', 1024); vi.stubGlobal('innerHeight', 640);
  scene.size = { width: 1024, height: 640 };
  scene.camera = new PerspectiveCamera(72, 1.6, .25, 6000);
  scene.controls = new OrbitControlsImpl(scene.camera);
  scene.controls.enableDamping = true;
  scene.gl = { domElement: document.createElement('canvas') };
  scene.props = null; scene.frame = null;
  useCommandCenter.setState({ daemonAuthError: null, chatDockOpen: false, chatLauncherSize: null });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount()); host.remove(); scene.controls.dispose();
  localStorage.clear(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});
function render(visible = true) { act(() => root.render(<ForumView visible={visible} />)); }
async function start(width: number, height: number) {
  resize(width, height); await settleStartup();
  expectVector(scene.camera.position, HABITAT_ARRIVAL.position);
  expectVector(scene.controls.target, HABITAT_ARRIVAL.target);
  expect(scene.camera.fov).toBe(72);
}
function resize(width: number, height: number) {
  scene.size = { width, height };
  scene.camera.aspect = width / Math.max(1, height); scene.camera.updateProjectionMatrix();
  render();
}
function click(text: string) {
  const button = [...host.querySelectorAll('button')].find(b => b.textContent === text)!;
  expect(button).toBeTruthy();
  act(() => { button.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true })); button.click(); });
}
function frames(seconds: number, count = 1) {
  act(() => { for (let i = 0; i < count; i++) {
    if (scene.controls.enabled) scene.controls.update(); // drei's priority -1 frame
    scene.frame({}, seconds / count);
  } });
}
function expectVector(actual: Vector3, expected: readonly number[]) {
  actual.toArray().forEach((value, i) => expect(value).toBeCloseTo(expected[i], 8));
}
function expectOverview() {
  const scale = Math.max(1, 1.6 / Math.max(.25, scene.size.width / scene.size.height));
  // Independent frozen contract, not a second call into the implementation.
  expectVector(scene.camera.position, [-3 + 33 * scale, -4 + 30 * scale, -16 + 58 * scale]);
  expectVector(scene.controls.target, [-3, -4, -16]);
  expect(scene.camera.fov).toBeCloseTo(50, 8);
}
function retainManualView(overview: boolean) {
  if (overview) { click('Overview'); frames(.85, 51); }
  act(() => scene.controls.dispatchEvent({ type: 'start' }));
  scene.camera.position.set(8, 7, 9); scene.controls.target.set(-2, 1, -6);
  scene.camera.fov = 61; scene.controls.update();
  leaveAndReturn();
  expectVector(scene.camera.position, [8, 7, 9]);
  expectVector(scene.controls.target, [-2, 1, -6]); expect(scene.camera.fov).toBe(61);
}
function leaveAndReturn() {
  const position = scene.camera.position.clone(), target = scene.controls.target.clone(), fov = scene.camera.fov;
  const size = scene.size;
  scene.size = { width: 0, height: 0 }; render(false); frames(2);
  scene.size = size; render();
  expectVector(scene.camera.position, position.toArray());
  expectVector(scene.controls.target, target.toArray()); expect(scene.camera.fov).toBe(fov);
}

// 1024x640 and 1280x800 both have aspect 1.6. The shell's sidebar/dock
// allocations produce the narrower cases; they are not window aspect ratios.
for (const [width, height] of [[1280, 800], [1024, 640], [784, 612], [464, 612]]) {
  it.each(['arrival', 'retained arrival', 'retained overview', 'retained place'])(
    `HUD Overview reaches the fitted contract within 0.85 s at canvas ${width}x${height} from %s`, async state => {
      await start(width, height);
      if (state === 'retained place') {
        const archive = [...host.querySelectorAll<HTMLButtonElement>('[role="radio"]')].find(b => b.textContent?.includes('Garden archive'))!;
        act(() => archive.click()); frames(.85, 51); leaveAndReturn();
      } else if (state !== 'arrival') retainManualView(state === 'retained overview');
      click('Overview');
      expect([...host.querySelectorAll('button')].find(b => b.textContent === 'Overview')?.getAttribute('aria-pressed')).toBe('true');
      frames(.85, 51); expectOverview();
      frames(3.65, 219); expectOverview(); // still correct at the native 4.5 s checkpoint
    });
}
it.each([[1024, 640], [784, 612], [464, 612]])(
  'HUD Overview meets the 0.85 s deadline with slow frames at canvas %sx%s', async (width, height) => {
    await start(width, height); retainManualView(false); click('Overview');
    frames(.8, 4); frames(.05); expectOverview();
  });
it.each([false, true])('resize during HUD Overview preserves its original deadline (retained=%s)', async retained => {
  await start(1024, 640);
  if (retained) retainManualView(true);
  click('Overview'); frames(.4, 24);
  const before = scene.camera.position.clone();
  resize(784, 612);
  expectVector(scene.camera.position, before.toArray());
  frames(.3, 18); resize(464, 612); frames(.15, 9);
  expectOverview();
});
it('repeated resize observations cannot strand HUD Overview near arrival', async () => {
  await start(784, 612); click('Overview');
  for (let i = 0; i < 17; i++) { resize(i % 2 ? 784 : 783, 612); frames(.05); }
  expectOverview();
});
it('pauses Overview while hidden and resumes the remaining travel with a newly fitted aspect', async () => {
  await start(1024, 640); click('Overview'); frames(.4, 24);
  const before = scene.camera.position.clone();
  scene.size = { width: 0, height: 0 }; render(false); frames(4);
  expectVector(scene.camera.position, before.toArray());
  resize(464, 612); frames(.45, 27); expectOverview();
});
it('finishes Overview on a delayed frame instead of advancing only 50 ms', async () => {
  await start(464, 612); click('Overview'); frames(.85); expectOverview();
});
it('a repeated explicit Overview gets a fresh deadline after resize retargeting', async () => {
  await start(1024, 640); click('Overview'); frames(.4, 24);
  resize(464, 612); click('Overview');
  frames(.45, 27);
  expect(scene.camera.fov).toBeGreaterThan(50);
  frames(.4, 24); expectOverview();
});
