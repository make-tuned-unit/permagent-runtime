/** @vitest-environment jsdom */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { PerspectiveCamera, Vector3 } from 'three';
import { MESH_GATE, gateLocalToWorld } from './meshPortal';

const scene = vi.hoisted(() => ({ frame: null as ((state: unknown, dt: number) => void) | null }));
const three = vi.hoisted(() => ({ camera: null as unknown, canvas: null as unknown }));
vi.mock('@react-three/fiber', () => ({
  useThree: () => ({ camera: three.camera, gl: { domElement: three.canvas } }),
  useFrame: (cb: (state: unknown, dt: number) => void) => { scene.frame = cb; },
}));
// The gate logic is what is under test, not the collision solver: this walks the
// step that was asked for, the way open floor would.
vi.mock('./walkCollision', () => ({
  attemptWalk: (feet: Vector3, dx: number, dz: number) => { feet.x += dx; feet.z += dz; return true; },
}));
import { ForumWalk } from './ForumWalk';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
beforeEach(() => {
  host = document.createElement('div');
  host.className = 'forum-shell';
  document.body.append(host);
  const canvas = document.createElement('canvas');
  host.append(canvas);
  three.canvas = canvas;
  three.camera = new PerspectiveCamera();
  root = createRoot(host.appendChild(document.createElement('div')));
});
afterEach(() => { act(() => root.unmount()); host.remove(); scene.frame = null; });

function frames(count: number) {
  act(() => { for (let i = 0; i < count; i++) scene.frame?.({}, 0.05); });
}
function press(code: string) { act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { code })); }); }

it('opens the Agora when the walker steps through the ring, and closes it walking back', () => {
  const onEnterMesh = vi.fn(), onExitMesh = vi.fn(), onExit = vi.fn();
  // Facing straight down -Z from in front of the ring, on the gate's centre
  // line: walking forward passes through the ring's opening.
  const start: [number, number, number] = [MESH_GATE.center[0], 0, MESH_GATE.center[2] + 4];
  act(() => root.render(<ForumWalk start={start} onExit={onExit} onInspect={vi.fn()} onEnterMesh={onEnterMesh} onExitMesh={onExitMesh} />));
  press('KeyW');
  frames(40); // 40 * 0.05s * 3.2m/s = 6.4m, past the plane 4m away
  expect(onEnterMesh).toHaveBeenCalledTimes(1);
  expect(onExit).not.toHaveBeenCalled(); // releasing pointer lock is not leaving walk mode

  // Now in the Agora: reverse back through the ring.
  act(() => root.render(<ForumWalk start={start} onExit={onExit} onInspect={vi.fn()} onEnterMesh={onEnterMesh} onExitMesh={onExitMesh} inMesh />));
  act(() => { window.dispatchEvent(new KeyboardEvent('keyup', { code: 'KeyW' })); });
  press('KeyS');
  frames(40);
  expect(onExitMesh).toHaveBeenCalledTimes(1);
  expect(onEnterMesh).toHaveBeenCalledTimes(1);
});

it('does not open the Agora for a step that passes wide of the ring', () => {
  const onEnterMesh = vi.fn();
  // Offset so the -Z path meets the ring plane 7 m off the centre line — past
  // the 4.6 m ring, i.e. round the side of the gate rather than through it.
  const start: [number, number, number] = [MESH_GATE.center[0] + 5, 0, MESH_GATE.center[2] + 3];
  act(() => root.render(<ForumWalk start={start} onExit={vi.fn()} onInspect={vi.fn()} onEnterMesh={onEnterMesh} />));
  press('KeyW');
  frames(70); // 11.2m — well past the 8m to the plane
  expect(onEnterMesh).not.toHaveBeenCalled();
});

it('offers E in the exedra and enters the Agora on that key', () => {
  const onEnterMesh = vi.fn(), onMeshPrompt = vi.fn();
  const [x, , z] = gateLocalToWorld([5.5, 0, 6]); // beyond the ring, inside the exedra
  act(() => root.render(<ForumWalk start={[x, 0, z]} onExit={vi.fn()} onInspect={vi.fn()} onEnterMesh={onEnterMesh} onMeshPrompt={onMeshPrompt} />));
  frames(1);
  expect(onMeshPrompt).toHaveBeenLastCalledWith(true);
  press('KeyE');
  expect(onEnterMesh).toHaveBeenCalledTimes(1);
});

it('leaves the Agora on Escape instead of leaving walk mode', () => {
  const onExit = vi.fn(), onExitMesh = vi.fn();
  act(() => root.render(<ForumWalk start={[0, 0, 0]} onExit={onExit} onInspect={vi.fn()} onExitMesh={onExitMesh} inMesh />));
  press('Escape');
  expect(onExitMesh).toHaveBeenCalledTimes(1);
  expect(onExit).not.toHaveBeenCalled();
});
