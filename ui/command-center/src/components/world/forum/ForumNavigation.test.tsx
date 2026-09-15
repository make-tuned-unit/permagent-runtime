/** @vitest-environment jsdom */
import { afterEach, expect, it, vi } from 'vitest';
import { createRoot } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { ensureMotion } from '../agents/motion';
import { ensureForumMotion } from './ForumInhabitants';
import { DISTRICTS } from './districts';
import { FORUM_VISTA, FORUM_BASE_FOV } from './vistaCamera';
const scene = vi.hoisted(() => ({
  // Enough of a PerspectiveCamera for the field-of-view switch: the vista and
  // the district focuses no longer share one lens.
  camera: { position: { set: vi.fn() }, isPerspectiveCamera: true, fov: 48, updateProjectionMatrix: vi.fn() },
  controls: { target: { set: vi.fn() }, update: vi.fn() },
}));
vi.mock('@react-three/fiber', () => ({ useThree: (select: (state: unknown) => unknown) => select({ camera: scene.camera }) }));
vi.mock('@react-three/drei', async () => {
  const React = await import('react');
  return {
    Html: () => null,
    OrbitControls: React.forwardRef((_, ref) => {
      React.useImperativeHandle(ref, () => scene.controls);
      return null;
    }),
  };
});
import { ForumNavigation } from './ForumNavigation';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => { vi.clearAllMocks(); scene.camera.fov = FORUM_BASE_FOV; });
it('finds the sovereign at its current walking position again on a repeated request', () => {
  const host = document.createElement('div');
  const root = createRoot(host);
  const live = ensureMotion('henry', { x: 0, y: 0, z: 0 });
  Object.assign(live, { x: 99, y: 0, z: 99 });
  const m = ensureForumMotion('henry', { x: 0, y: 0, z: 0 });
  Object.assign(m, { x: 5, y: 0, z: 3 });
  act(() => root.render(<ForumNavigation district={DISTRICTS[0]} onDistrict={() => {}} focusAgent={{ id: 'henry', revision: 1 }} />));
  expect(scene.camera.position.set).toHaveBeenLastCalledWith(10, 3.5, 10);
  expect(scene.controls.target.set).toHaveBeenLastCalledWith(5, 1.2, 3);
  Object.assign(live, { x: -99, y: 0, z: -99 });
  Object.assign(m, { x: -2, y: 4.32, z: 8 });
  act(() => root.render(<ForumNavigation district={DISTRICTS[0]} onDistrict={() => {}} focusAgent={{ id: 'henry', revision: 2 }} />));
  expect(scene.camera.position.set).toHaveBeenLastCalledWith(3, 7.82, 15);
  expect(scene.controls.target.set).toHaveBeenLastCalledWith(-2, 5.5200000000000005, 8);
  act(() => root.unmount());
});

it('opens the commons on the wide vista, and gives every other district back the narrow lens', () => {
  const host = document.createElement('div');
  const root = createRoot(host);
  act(() => root.render(<ForumNavigation district={DISTRICTS[0]} onDistrict={() => {}} focusAgent={null} />));
  expect(DISTRICTS[0].id).toBe('commons');
  expect(scene.camera.position.set).toHaveBeenLastCalledWith(...FORUM_VISTA.position);
  expect(scene.controls.target.set).toHaveBeenLastCalledWith(...FORUM_VISTA.target);
  expect(scene.camera.fov).toBe(FORUM_VISTA.fov);
  expect(scene.controls.update).toHaveBeenCalled();

  const gallery = DISTRICTS.find(d => d.id === 'gallery')!;
  act(() => root.render(<ForumNavigation district={gallery} onDistrict={() => {}} focusAgent={null} />));
  expect(scene.camera.fov).toBe(FORUM_BASE_FOV);
  expect(scene.camera.position.set).toHaveBeenLastCalledWith(gallery.position[0] + 13, gallery.position[1] + 12, gallery.position[2] + 16);
  act(() => root.unmount());
});

it('drops back to the narrow lens to find a person, so a portrait is not shot on a landscape field', () => {
  const host = document.createElement('div');
  const root = createRoot(host);
  const person = ensureForumMotion('librarian', { x: 0, y: 0, z: 0 });
  Object.assign(person, { x: 4, y: 0, z: -6 });
  act(() => root.render(<ForumNavigation district={DISTRICTS[0]} onDistrict={() => {}} focusAgent={{ id: 'librarian', revision: 1 }} />));
  expect(scene.camera.fov).toBe(FORUM_BASE_FOV);
  expect(scene.camera.position.set).toHaveBeenLastCalledWith(9, 3.5, 1);
  act(() => root.unmount());
});
