/**
 * @vitest-environment jsdom
 *
 * "There is no liquid glass anywhere" — the rail's material.
 *
 * The rail used to paint `gradient.sidebar`, a one-off tint no other surface
 * shared and one that ignored Reduce Transparency. It now wears `glassHi`,
 * the token the design system names for sidebars, through `useGlass`.
 *
 * WHAT THIS PROVES: the rail asks for the right material (glassHi's
 * translucent fill and rim), collapses to the flat surface under Reduce
 * Transparency, and does NOT ask for a CSS blur.
 * WHAT IT CANNOT PROVE: how it looks. jsdom has no compositor and no
 * NSWindow. The main window is deliberately opaque (chrome.rs, shell.test.ts:
 * transparency measured at +6 points idle GPU), so in the real app nothing
 * shows through the rail; it reads as glass by its rim and depth only.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Sidebar } from './Sidebar';
import { useCommandCenter } from '../../lib/store';
import { THEME_GLASS, getThemedColors, setNativeReduceTransparency, setTheme } from '../../styles/tokens';
import { ViewHeader } from '../common/ViewHeader';

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
  setTheme('dark');
  setNativeReduceTransparency(false);
});

/** A colour as jsdom's style object reads it back (`rgba(1, 2, 3, 0.5)`). */
function normalised(cssColor: string): string {
  const probe = document.createElement('div');
  probe.style.background = cssColor;
  return probe.style.background;
}

function rail(): HTMLElement {
  act(() => root.render(<Sidebar />));
  const el = container.firstElementChild as HTMLElement | null;
  expect(el, 'Sidebar rendered nothing').toBeTruthy();
  return el!;
}

describe('the sidebar rail wears glassHi', () => {
  it('paints the glassHi translucent fill and rim, per theme', () => {
    for (const theme of ['dark', 'silver'] as const) {
      setTheme(theme);
      const el = rail();
      const token = THEME_GLASS[theme].glassHi;
      expect(el.style.background, theme).toBe(normalised(token.background));
      expect(el.style.background, `${theme} fill must be translucent`).toMatch(/^rgba\(.*, 0\.\d+\)$/);
      expect(el.style.boxShadow, theme).not.toBe('');
      // The rim's inset hairline is the seam; a borderRight beside it doubled it.
      expect(el.style.borderRight, theme).toBe('');
    }
  });

  it('never asks for a CSS blur — the rail is flush, so the blur is the window\'s', () => {
    // Nothing in the web layer can sit behind a rail flush to the window edge,
    // so a backdrop-filter here would only blur the shell's flat fill into
    // itself: a full compositing pass for zero pixels.
    const el = rail();
    expect(el.style.backdropFilter).toBeFalsy();
    expect((el.style as unknown as Record<string, unknown>).WebkitBackdropFilter).toBeFalsy();
  });

  it('goes to the flat surface under Reduce Transparency', () => {
    setNativeReduceTransparency(true);
    const el = rail();
    expect(el.style.background).toBe(normalised(THEME_GLASS.dark.glassHi.opaque));
  });
});

// ── One seam, not two ──
// The rail and the header both wore the token's rim, whose hairline is an
// `inset 0 0 0 1px` RING — four sides. Docked side by side, the header's left
// ring sat against the rail's right ring: a 2px rail/content seam in the header
// band and 1px below it (the Home QA screenshot checked only below the header).
// The ring also drew lines down the window frame on the rail's left, top and
// bottom, and under the titlebar band on the header's top.
//
// WHAT THIS PROVES: each docked surface asks for exactly one hairline, on its
// seam (the rail's right, the header's bottom), in every theme, and neither
// asks for the four-sided ring. WHAT IT CANNOT PROVE: the pixels. jsdom does
// not paint, so "one 1px line on screen" is still the real-app check.
describe('the rail and the header share one seam', () => {
  const RING = /inset 0 0 0 1px/;

  function header(): HTMLElement {
    act(() => root.render(<ViewHeader title="Home" />));
    const el = container.querySelector('[data-testid="view-header"]') as HTMLElement | null;
    expect(el, 'ViewHeader rendered nothing').toBeTruthy();
    return el!;
  }

  for (const theme of ['dark', 'aurora', 'silver'] as const) {
    it(`draws the hairline on the rail's right edge only (${theme})`, () => {
      setTheme(theme);
      const shadow = rail().style.boxShadow;
      expect(shadow).not.toMatch(RING);
      // One hairline, on the seam, in the theme's own border colour.
      expect(shadow).toContain(`inset -1px 0 0 0 ${getThemedColors().border}`);
    });

    it(`draws the header's hairline on its bottom edge only (${theme})`, () => {
      setTheme(theme);
      const shadow = header().style.boxShadow;
      expect(shadow).not.toMatch(RING);
      // No left-edge hairline beside the rail's right one.
      expect(shadow).not.toContain('inset 1px 0');
      expect(shadow).toContain(`inset 0 -1px 0 0 ${getThemedColors().border}`);
    });
  }

  it('keeps the seam under Reduce Transparency: that setting drops the see-through, not the edges', () => {
    setNativeReduceTransparency(true);
    expect(rail().style.boxShadow).toContain('inset -1px 0');
  });
});
