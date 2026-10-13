/** @vitest-environment jsdom */
import { Suspense, act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const gpu = vi.hoisted(() => ({ props: null as Record<string, any> | null }));
vi.mock('@react-three/fiber', () => ({ Canvas: (props: Record<string, any>) => { gpu.props = props; return <canvas />; }, useFrame: () => {}, useThree: () => () => {} }));
import { HeroAgentPortrait } from './HeroAgentPortrait';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// Native r35 (scene-recovery receipt): with the hero kit GLB stalled, the dock portrait's Canvas
// suspended, R3F rethrew it to the DOM tree, and the World route's fallback replaced the whole
// World (HUD, stage status and Retry) with a bare "Loading World…" that never offered recovery.
it('keeps a stalled hero kit inside the portrait Canvas', () => {
  const host = document.createElement('div'); const root = createRoot(host);
  act(() => root.render(<HeroAgentPortrait id="henry" size="dock" />));
  expect(gpu.props).toBeTruthy();
  const child = gpu.props!.children;
  expect(child?.type).toBe(Suspense);
  expect(child.props.fallback).toBeNull();
  expect(child.props.children.type.name).toBe('PortraitScene');
  act(() => root.unmount());
});
