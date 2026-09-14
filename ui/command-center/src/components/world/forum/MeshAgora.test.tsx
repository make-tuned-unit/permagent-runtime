/** @vitest-environment jsdom */
import { afterAll, afterEach, beforeAll, expect, it, vi } from 'vitest';
import { createRoot, Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import type { MeshStatus } from '../shared/meshStatus';

const status = vi.hoisted(() => ({ value: { state: 'offline' } as MeshStatus }));
vi.mock('../shared/meshStatus', () => ({ useMeshStatus: () => status.value }));
vi.mock('@react-three/fiber', () => ({
  useFrame: () => {},
  useThree: (select?: (s: unknown) => unknown) => {
    const state = { camera: { position: { set: () => {} }, lookAt: () => {} } };
    return select ? select(state) : state;
  },
}));
vi.mock('@react-three/drei', async () => {
  const React = await import('react');
  return {
    Html: ({ children }: { children?: React.ReactNode }) => <div>{children}</div>,
    OrbitControls: React.forwardRef(() => null),
  };
});
// InstancedProp writes matrices straight onto an InstancedMesh; there is no
// renderer here, so the chevrons and steles are stubbed out.
vi.mock('../shared/instancing', () => ({ InstancedProp: () => null }));
import { MeshAgora } from './MeshAgora';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
// react-three-fiber's intrinsic elements (<mesh/>, <torusGeometry/> …) are not
// DOM elements, so react-dom warns about each one. The room's geometry is not
// what this file is testing; keep the real console for anything else.
const realError = console.error;
beforeAll(() => {
  console.error = (...args: unknown[]) => {
    if (typeof args[0] === 'string' && args[0].includes('is using incorrect casing')) return;
    realError(...(args as []));
  };
});
afterAll(() => { console.error = realError; });

let root: Root | undefined, host: HTMLDivElement | undefined;
function render() {
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  act(() => root!.render(<MeshAgora walking />));
  return host;
}
afterEach(() => {
  if (root) act(() => root!.unmount());
  host?.remove();
  root = undefined;
  status.value = { state: 'offline' };
});

it('says the Mesh is not connected while the shared status is offline', () => {
  expect(render().textContent).toContain('MESH: NOT CONNECTED');
});

it('says the Mesh is connecting while the shared status is connecting', () => {
  status.value = { state: 'connecting' };
  expect(render().textContent).toContain('MESH: CONNECTING');
});

it('prints the real peer count only when the shared status is connected', () => {
  status.value = { state: 'connected', peerCount: 3 };
  const text = render().textContent ?? '';
  expect(text).toContain('MESH: CONNECTED · 3 peers');
  expect(text).not.toContain('NOT CONNECTED');
  expect(host!.querySelector('[data-testid="mesh-plaque"]')?.getAttribute('data-state')).toBe('connected');
});

it('speaks of a single peer in the singular', () => {
  status.value = { state: 'connected', peerCount: 1 };
  expect(render().textContent).toContain('MESH: CONNECTED · 1 peer');
});
