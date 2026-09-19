/**
 * @vitest-environment jsdom
 *
 * "There is no liquid glass anywhere" — the rail over a TRANSPARENT window.
 *
 * A review of the first fix caught that even once the window is made
 * transparent (macOS `sidebar` material behind the web layer), glassHi's 90%
 * fill would paint over nearly all of it. The rail therefore keys its fill off
 * the same `NATIVE_WINDOW_IS_OPAQUE` flag App.tsx reads, and thins to
 * `RAIL_FILL_OVER_VIBRANCY` when that flag is false.
 *
 * WHAT THIS PROVES: with the flag flipped, the rail paints the thin per-theme
 * tint (and nothing more opaque than glassHi); with the flag as shipped, it
 * keeps glassHi; Reduce Transparency still collapses it to the flat surface.
 * WHAT IT CANNOT PROVE: that the wallpaper actually shows through. That needs
 * the transparent NSWindow, which jsdom does not have and which ships only
 * after the GPU re-measurement and sign-off.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const windowFlag = vi.hoisted(() => ({ opaque: true }));
vi.mock('../../lib/windowChrome', async (importOriginal) => {
  const real = await importOriginal<typeof import('../../lib/windowChrome')>();
  return {
    ...real,
    get NATIVE_WINDOW_IS_OPAQUE() { return windowFlag.opaque; },
  };
});

import { Sidebar } from './Sidebar';
import { useCommandCenter } from '../../lib/store';
import {
  RAIL_FILL_OVER_VIBRANCY, THEME_GLASS, setNativeReduceTransparency, setTheme,
} from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => { useCommandCenter.setState({ activePanel: 'chat' }); });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  windowFlag.opaque = true;
  setTheme('dark');
  setNativeReduceTransparency(false);
});

function normalised(cssColor: string): string {
  const probe = document.createElement('div');
  probe.style.background = cssColor;
  return probe.style.background;
}

function alpha(cssColor: string): number {
  const m = /rgba\([^)]*,\s*([0-9.]+)\)/.exec(cssColor);
  if (!m) throw new Error(`not an rgba colour: ${cssColor}`);
  return Number(m[1]);
}

function rail(): HTMLElement {
  act(() => root.render(<Sidebar />));
  const el = container.firstElementChild as HTMLElement | null;
  expect(el, 'Sidebar rendered nothing').toBeTruthy();
  return el!;
}

describe('the rail over a transparent window', () => {
  it('thins to the vibrancy tint so the native material is not buried', () => {
    windowFlag.opaque = false;
    for (const theme of ['dark', 'aurora', 'silver'] as const) {
      setTheme(theme);
      const el = rail();
      expect(el.style.background, theme).toBe(normalised(RAIL_FILL_OVER_VIBRANCY[theme]));
      // The invariant the review asked for: strictly thinner than glassHi,
      // and still a tint (not fully clear), so labels keep a ground.
      const a = alpha(el.style.background);
      expect(a, theme).toBeLessThan(alpha(THEME_GLASS[theme].glassHi.background));
      expect(a, theme).toBeGreaterThan(0);
      // Still no CSS blur: the blur is the NSVisualEffectView's.
      expect(el.style.backdropFilter, theme).toBeFalsy();
    }
  });

  it('keeps glassHi while the window is opaque, as shipped', () => {
    const el = rail();
    expect(el.style.background).toBe(normalised(THEME_GLASS.dark.glassHi.background));
  });

  it('still goes flat under Reduce Transparency, transparent window or not', () => {
    windowFlag.opaque = false;
    setNativeReduceTransparency(true);
    const el = rail();
    expect(el.style.background).toBe(normalised(THEME_GLASS.dark.glassHi.opaque));
  });
});
