/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { PerspectiveCamera, Vector3 } from 'three';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const scene = vi.hoisted(() => ({ camera: null as any, gl: null as any, frame: null as any }));
vi.mock('@react-three/fiber', () => ({ useThree: () => scene, useFrame: (callback: unknown) => { scene.frame = callback; } }));
vi.mock('./walkCollision', () => ({ attemptWalk: (feet: Vector3, dx: number, dz: number) => { feet.x += dx; feet.z += dz; return true; } }));
import { ForumWalk } from './ForumWalk';
import { FORUM_WALK_FOV } from './vistaCamera';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
let lockDescriptor: PropertyDescriptor | undefined, exitDescriptor: PropertyDescriptor | undefined;
const start: [number, number, number] = [0, 0, 6];
const onExit = vi.fn(), onInspect = vi.fn();
beforeEach(() => {
  lockDescriptor = Object.getOwnPropertyDescriptor(document, 'pointerLockElement');
  exitDescriptor = Object.getOwnPropertyDescriptor(document, 'exitPointerLock');
  host = document.createElement('div'); host.className = 'forum-shell'; document.body.append(host);
  const canvas = document.createElement('canvas'); host.append(canvas);
  scene.camera = new PerspectiveCamera(); scene.gl = { domElement: canvas };
  root = createRoot(host.appendChild(document.createElement('div')));
  onExit.mockReset(); onInspect.mockReset();
});
afterEach(() => {
  act(() => root.unmount()); host.remove(); vi.restoreAllMocks();
  for (const [key, descriptor] of [['pointerLockElement', lockDescriptor], ['exitPointerLock', exitDescriptor]] as const) {
    if (descriptor) Object.defineProperty(document, key, descriptor); else Reflect.deleteProperty(document, key);
  }
});
function render(active?: boolean, from = start) { act(() => root.render(<ForumWalk start={from} active={active} onExit={onExit} onInspect={onInspect} />)); }
function key(code: string) {
  const event = new KeyboardEvent('keydown', { code, key: code === 'Escape' ? 'Escape' : undefined, bubbles: true, cancelable: true });
  act(() => document.body.dispatchEvent(event)); return event;
}
function frames(count = 1) { act(() => { for (let i = 0; i < count; i++) scene.frame({}, .05); }); }

it('releases hidden walk input and pointer lock while preserving feet, pitch, yaw and FOV on resume', () => {
  render();
  let locked: Element | null = scene.gl.domElement;
  Object.defineProperty(document, 'pointerLockElement', { configurable: true, get: () => locked });
  Object.defineProperty(document, 'exitPointerLock', { configurable: true, value: vi.fn(() => {
    locked = null; document.dispatchEvent(new Event('pointerlockchange'));
  }) });
  act(() => document.dispatchEvent(new Event('pointerlockchange')));
  key('KeyW'); key('ArrowLeft'); frames(5);
  act(() => {
    const move = new MouseEvent('mousemove');
    Object.defineProperties(move, { movementX: { value: 18 }, movementY: { value: -30 } });
    document.dispatchEvent(move);
  });
  const position = scene.camera.position.clone(), quaternion = scene.camera.quaternion.clone();
  expect(position.toArray()).not.toEqual([0, 1.7, 6]);
  render(false);
  expect(document.exitPointerLock).toHaveBeenCalledOnce();
  for (const code of ['KeyW', 'ArrowLeft', 'Escape', 'KeyE']) expect(key(code).defaultPrevented).toBe(false);
  act(() => scene.gl.domElement.dispatchEvent(new MouseEvent('mousedown')));
  frames(5);
  expect(scene.camera.position.equals(position)).toBe(true);
  expect(scene.camera.quaternion.equals(quaternion)).toBe(true);
  expect(onExit).not.toHaveBeenCalled(); expect(onInspect).not.toHaveBeenCalled();
  render(true); frames(3);
  expect(scene.camera.position.equals(position)).toBe(true); // held keys were cleared
  expect(scene.camera.quaternion.equals(quaternion)).toBe(true);
  expect(scene.camera.fov).toBe(FORUM_WALK_FOV);
  expect(key('KeyW').defaultPrevented).toBe(true); frames();
  expect(scene.camera.position.distanceTo(position)).toBeCloseTo(.16);
  expect(scene.camera.quaternion.equals(quaternion)).toBe(true);
  key('Escape'); expect(onExit).toHaveBeenCalledOnce();
});

it('retains default placement on ordinary remounts and explicit start changes', () => {
  render(); key('KeyW'); key('ArrowRight'); frames(4);
  act(() => root.render(null)); render();
  expect(scene.camera.position.toArray()).toEqual([0, 1.7, 6]);
  expect(scene.camera.getWorldDirection(new Vector3()).distanceTo(new Vector3(0, 0, -1))).toBe(0);
  render(undefined, [2, 3.4, -8]);
  expect(scene.camera.position.toArray()).toEqual([2, 5.1, -8]);
  expect(scene.camera.fov).toBe(FORUM_WALK_FOV);
});
