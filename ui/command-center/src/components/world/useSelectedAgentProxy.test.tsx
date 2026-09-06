// @vitest-environment jsdom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentState } from './types';

const fixture = vi.hoisted(() => ({
  frame: (() => {}) as () => void,
  positions: new Map<string, { x: number; y: number; z: number }>([
    ['henry', { x: 1, y: 0, z: 2 }],
    ['reader', { x: 5, y: 0, z: 6 }],
  ]),
}));
vi.mock('@react-three/fiber', () => ({ useFrame: (callback: () => void) => { fixture.frame = callback; } }));
vi.mock('./agents', () => ({
  ROSTER: [
    { id: 'henry', name: 'Henry', role: 'companion', trimColor: '#fff', isHenry: true },
    { id: 'reader', name: 'Reader', role: 'reader', trimColor: '#aaa', isHenry: false },
  ],
  getAgentPosition: (id: string) => fixture.positions.get(id),
}));
import { useSelectedAgentProxy } from './useSelectedAgentProxy';

describe('selected agent camera binding', () => {
  let dispose: (() => void) | undefined;
  afterEach(() => { dispose?.(); dispose = undefined; vi.unstubAllGlobals(); });

  it('delivers selection immediately and follows motion without an unrelated render', () => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
    const host = document.createElement('div');
    const root = createRoot(host);
    dispose = () => act(() => root.unmount());
    let current: AgentState | null = null;
    function Harness({ id }: { id: string | null }) {
      current = useSelectedAgentProxy(id);
      return null;
    }
    act(() => root.render(<Harness id={null} />));
    expect(current).toBeNull();
    act(() => root.render(<Harness id="henry" />));
    expect(current).toMatchObject({ id: 'henry', position: { x: 1, y: 0, z: 2 } });
    const selected = current;
    fixture.positions.set('henry', { x: 8, y: 1, z: 9 });
    fixture.frame();
    expect(current).toBe(selected);
    expect(current).toMatchObject({ position: { x: 8, y: 1, z: 9 } });
    act(() => root.render(<Harness id="reader" />));
    expect(current).not.toBe(selected);
    expect(current).toMatchObject({ id: 'reader', position: { x: 5, y: 0, z: 6 } });
    act(() => root.render(<Harness id={null} />));
    fixture.frame();
    expect(current).toBeNull();
    act(() => root.render(<Harness id="unknown" />));
    expect(current).toBeNull();
  });
});
