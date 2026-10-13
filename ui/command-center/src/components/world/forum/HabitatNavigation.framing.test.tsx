/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { PerspectiveCamera, Vector3 } from 'three';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const scene = vi.hoisted(() => ({ camera: null as any, controls: null as any, size: { width: 1280, height: 800 }, frame: null as any, onStart: null as any }));
vi.mock('@react-three/fiber', () => ({ useThree: (pick: (s: typeof scene) => unknown) => pick(scene), useFrame: (frame: unknown) => { scene.frame = frame; } }));
vi.mock('@react-three/drei', async original => {
  const React = await import('react');
  return { ...await original<any>(), OrbitControls: React.forwardRef((_props: any, ref) => {
    scene.onStart = _props.onStart; React.useImperativeHandle(ref, () => scene.controls); return null;
  }) };
});
import { HabitatNavigation } from './HabitatScene';
import { HABITAT_ARRIVAL, habitatOverviewCamera } from './habitatLayout';
import { setReduceMotion } from '../../../styles/tokens';
import { ForumActivity } from './ForumActivity';
import { DISTRICTS } from './districts';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement;
beforeEach(() => {
  localStorage.clear(); setReduceMotion(false);
  scene.camera = new PerspectiveCamera(48, 1.6, .25, 6000);
  scene.controls = { target: new Vector3(), update: () => scene.camera.lookAt(scene.controls.target) };
  scene.size = { width: 1280, height: 800 };
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); localStorage.clear(); });
function render(overview = true, active = true) {
  act(() => root.render(<ForumActivity.Provider value={active}><HabitatNavigation district={DISTRICTS[0]} focusAgent={null} overview={overview} follow={null} /></ForumActivity.Provider>));
}
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
