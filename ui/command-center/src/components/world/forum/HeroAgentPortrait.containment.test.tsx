/** @vitest-environment jsdom */
import { Suspense, act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const gpu = vi.hoisted(() => ({ props: null as Record<string, any> | null, fail: false }));
vi.mock('@react-three/fiber', () => ({ Canvas: (props: Record<string, any>) => { if (gpu.fail) throw new Error('hero kit failed'); gpu.props = props; return <canvas />; }, useFrame: () => {}, useThree: () => () => {} }));
import { HeroAgentPortrait } from './HeroAgentPortrait';
import { clearUnsettledForumAssets } from './forumAssetRetry';
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

// Native r36 (scene-recovery receipt): after a failed hero kit, Retry World loaded the scene
// (Aria and the garden came back) but the dock emblem stayed on the "◇ / Portrait unavailable"
// fallback, because the portrait's failure boundary lives outside the stage Canvas that Retry remounts.
it('gives a failed portrait a fresh attempt on Retry World', () => {
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
  const host = document.createElement('div'); const root = createRoot(host);
  gpu.fail = true;
  act(() => root.render(<HeroAgentPortrait id="henry" size="profile" label="Henry" />));
  const portrait = () => host.querySelector('[data-testid="live-agent-portrait"]')!;
  expect(portrait().getAttribute('aria-label')).toBe('Portrait unavailable');
  expect(host.querySelector('canvas')).toBeNull();
  act(() => root.render(<HeroAgentPortrait id="henry" size="profile" label="Henry" />));
  expect(portrait().getAttribute('aria-label')).toBe('Portrait unavailable'); // a re-render alone keeps the latch
  gpu.fail = false;
  act(() => { clearUnsettledForumAssets(); });
  expect(portrait().getAttribute('aria-label')).toBe('Henry');
  expect(host.querySelector('canvas')).not.toBeNull();
  act(() => root.unmount()); errors.mockRestore();
});
