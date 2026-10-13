/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { PerspectiveCamera, Vector3 } from 'three';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const scene = vi.hoisted(() => ({ camera: null as any, controls: null as any, gl: {}, setEvents: () => {}, size: { width: 1280, height: 800 }, frame: null as any, onStart: null as any, props: null as any }));
vi.mock('@react-three/fiber', () => ({ useThree: (pick: (s: typeof scene) => unknown) => pick(scene), useFrame: (frame: unknown) => { scene.frame = frame; } }));
vi.mock('@react-three/drei', async original => {
  const React = await import('react');
  return { ...await original<any>(), OrbitControls: React.forwardRef((_props: any, ref) => {
    // R3F applies changed control props before layout-effect cleanup/setup.
    if (scene.props?.enabled !== _props.enabled) scene.controls.enabled = _props.enabled;
    scene.props = _props; scene.onStart = _props.onStart; React.useImperativeHandle(ref, () => scene.controls); return null;
  }) };
});
import { HabitatNavigation } from './HabitatScene';
import { HABITAT_ARRIVAL, HABITAT_PLACES, TABLE_CONVERSATION_CAMERA, habitatOverviewCamera } from './habitatLayout';
import { setReduceMotion } from '../../../styles/tokens';
import { ForumActivity, ForumCanvasActivity } from './ForumActivity';
import { DISTRICTS } from './districts';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement;
beforeEach(() => {
  localStorage.clear(); setReduceMotion(false);
  scene.camera = new PerspectiveCamera(48, 1.6, .25, 6000);
  scene.controls = { target: new Vector3(), update: () => scene.camera.lookAt(scene.controls.target) };
  scene.props = null;
  scene.size = { width: 1280, height: 800 };
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); localStorage.clear(); });
function render(overview = true, active = true, options: Partial<React.ComponentProps<typeof HabitatNavigation>> = {}) {
  act(() => root.render(<ForumActivity.Provider value={active}><ForumCanvasActivity active={active} /><HabitatNavigation district={DISTRICTS[0]} focusAgent={null} overview={overview} follow={null} {...options} /></ForumActivity.Provider>));
}
it.each([[1280, 800], [1024, 640], [464, 640]])('uses the arrival contract on entry at %sx%s without aspect-fitting it', (width, height) => {
  scene.size = { width, height }; render(false);
  expect(scene.camera.position.toArray()).toEqual(HABITAT_ARRIVAL.position);
  expect(scene.controls.target.toArray()).toEqual(HABITAT_ARRIVAL.target);
  expect(scene.camera.fov).toBe(72);
});
it('applies the exported overview position, target and FOV to the actual camera on first paint', () => {
  render(); const view = habitatOverviewCamera(1.6);
  expect(scene.camera.position.toArray()).toEqual(view.position);
  expect(scene.controls.target.toArray()).toEqual(view.target); expect(scene.camera.fov).toBe(view.fov);
});
it('fits a narrower canvas without changing the frozen target or FOV', () => {
  setReduceMotion(true); render(); scene.size = { width: 640, height: 800 }; render();
  const view = habitatOverviewCamera(.8);
  expect(scene.camera.position.toArray()).toEqual(view.position);
  expect(scene.controls.target.toArray()).toEqual(view.target); expect(scene.camera.fov).toBe(50);
});
it('applies keyboard overview immediately instead of starting camera travel', () => {
  render(false); expect(scene.camera.fov).toBe(HABITAT_ARRIVAL.fov);
  act(() => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }))); render(true);
  expect(scene.camera.position.toArray()).toEqual(habitatOverviewCamera().position);
});
it('interrupts pointer travel and snaps when reduced motion is enabled mid-travel', () => {
  render(false); render(true); scene.frame({}, .05);
  const interrupted = scene.camera.position.toArray(); scene.onStart(); scene.frame({}, .05);
  expect(scene.camera.position.toArray()).toEqual(interrupted);
  render(false); render(true); scene.frame({}, .05);
  act(() => setReduceMotion(true));
  expect(scene.camera.position.toArray()).toEqual(habitatOverviewCamera().position);
});
it.each([false, true])('keeps the manually orbited camera across a zero-size hidden canvas and return (overview=%s)', overview => {
  render(overview); scene.onStart();
  scene.camera.position.set(8, 7, 9); scene.controls.target.set(-2, 1, -6); scene.camera.fov = 61;
  scene.size = { width: 0, height: 0 }; render(overview, false);
  expect(scene.controls.enabled).toBe(false);
  scene.size = { width: 1024, height: 640 }; render(overview);
  expect(scene.camera.position.toArray()).toEqual([8, 7, 9]);
  expect(scene.controls.target.toArray()).toEqual([-2, 1, -6]); expect(scene.camera.fov).toBe(61);
  expect(scene.controls.enabled).toBe(true);
});
it('keeps the selected place camera across workspace return and a desk resize', () => {
  render(false, true, { district: HABITAT_PLACES[1] });
  const position = scene.camera.position.clone(), target = scene.controls.target.clone();
  scene.size = { width: 0, height: 0 }; render(false, false, { district: HABITAT_PLACES[1] });
  scene.size = { width: 432, height: 500 }; render(false, true, { district: HABITAT_PLACES[1] });
  expect(scene.camera.position.toArray()).toEqual(position.toArray()); expect(scene.controls.target.toArray()).toEqual(target.toArray());
});
it('returns a manually orbited overview to the fitted contract on a repeated explicit action', () => {
  setReduceMotion(true); render(); scene.onStart(); scene.camera.position.set(2, 3, 4);
  render(true, true, { overviewRevision: 1 });
  expect(scene.camera.position.toArray()).toEqual(habitatOverviewCamera().position);
});
it('retains the table camera and frozen clipping and orbit limits', () => {
  render(false, true, { conversation: true });
  expect(scene.camera.position.toArray()).toEqual(TABLE_CONVERSATION_CAMERA.position);
  expect(scene.controls.target.toArray()).toEqual(TABLE_CONVERSATION_CAMERA.target); expect(scene.camera.fov).toBe(34);
  expect(scene.camera.near).toBe(.25); expect(scene.camera.far).toBe(6000);
  expect(scene.props).toMatchObject({ enabled: false, minDistance: 1, maxDistance: 110, maxPolarAngle: Math.PI * .51 });
  scene.size = { width: 640, height: 800 }; render(false, true, { conversation: true });
  expect(scene.props.maxDistance).toBe(220);
});
it('does not advance a camera transition while World is hidden', () => {
  render(false); render(true); scene.frame({}, .05); const position = scene.camera.position.clone();
  render(true, false); scene.frame({}, .05); expect(scene.camera.position.toArray()).toEqual(position.toArray());
  render(true); scene.frame({}, .05); expect(scene.camera.position.toArray()).not.toEqual(position.toArray());
});
