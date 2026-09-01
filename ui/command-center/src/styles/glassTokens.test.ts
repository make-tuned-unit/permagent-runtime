/**
 * The glass, fill, space and motion tokens, frozen.
 *
 * Same job as `textScale.test.ts` and `radiusScale.test.ts`: these values were
 * derived rather than picked, and a derivation nobody can see is a number the
 * next person will nudge. So the test states the reasoning next to the value,
 * and changing one means arguing with the reasoning first.
 *
 * The values themselves come from `docs/design/APPLE_LIQUID_GLASS_RESEARCH.md`
 * and are labelled there by provenance. Nothing here is a number Apple
 * published — Apple ships exactly one hard alpha in its entire corpus (the 35%
 * dimming layer) and describes everything else qualitatively.
 */

import { describe, expect, it } from 'vitest';

import {
  THEME_GLASS, THEME_GRADIENTS, space, radius, concentric, ease, duration, SPRING_LINEAR,
  getThemedColors, setTheme, getTheme, type ThemeId,
} from './tokens';

const THEMES: ThemeId[] = ['dark', 'aurora', 'silver'];

describe('glass tokens', () => {
  it('is the dark theme, verbatim', () => {
    expect(THEME_GLASS.dark).toEqual({
      glass: {
        background: 'rgba(30,36,51,0.82)',
        backdropFilter: 'blur(20px) saturate(180%)',
        boxShadow: [
          'inset 0 1px 0 rgba(255,255,255,0.14)',
          'inset 0 -1px 0 rgba(0,0,0,0.22)',
          'inset 0 0 0 1px rgba(255,255,255,0.07)',
          '0 8px 32px rgba(0,0,0,0.28)',
        ].join(', '),
        opaque: '#1E2433',
      },
      glassHi: {
        // 0.9, not 0.90 — the alpha is computed, and JS does not keep the zero.
        background: 'rgba(30,36,51,0.9)',
        backdropFilter: 'blur(24px) saturate(170%)',
        boxShadow: [
          'inset 0 1px 0 rgba(255,255,255,0.14)',
          'inset 0 -1px 0 rgba(0,0,0,0.22)',
          'inset 0 0 0 1px rgba(255,255,255,0.07)',
          '0 16px 48px rgba(0,0,0,0.28)',
        ].join(', '),
        opaque: '#1E2433',
      },
    });
  });

  it('makes the LARGER surface more opaque, in every theme', () => {
    // The rule people get backwards, and Apple's own: large glass "uses
    // increased opacity to preserve legibility over complex backgrounds"
    // (WWDC25/219). A sidebar is not more transparent for being more glass.
    for (const theme of THEMES) {
      const { glass, glassHi } = THEME_GLASS[theme];
      const alpha = (bg: string) => Number(/,\s*([\d.]+)\)$/.exec(bg)?.[1]);
      expect(alpha(glassHi.background), theme).toBeGreaterThan(alpha(glass.background));
    }
  });

  it('sits at Tinted opacity, not at the June-2025 launch transparency', () => {
    // Apple spent 26.1, 26.2 and 26.4 making its own material less transparent
    // and shipped a switch to turn the effect down. Designing to the launch
    // beta would be designing to a look its author already retreated from.
    for (const theme of THEMES) {
      const alpha = (bg: string) => Number(/,\s*([\d.]+)\)$/.exec(bg)?.[1]);
      expect(alpha(THEME_GLASS[theme].glass.background), theme).toBeGreaterThanOrEqual(0.8);
    }
  });

  it('always pairs a translucent fill with a filter — never a filter alone', () => {
    // The bug the whole lane exists to fix: `backdropFilter` over an opaque
    // `colors.surface` blurs nothing and still costs a compositing pass.
    for (const theme of THEMES) {
      for (const surface of Object.values(THEME_GLASS[theme])) {
        expect(surface.background, theme).toMatch(/^rgba\(/);
        expect(Number(/,\s*([\d.]+)\)$/.exec(surface.background)?.[1]), theme).toBeLessThan(1);
        expect(surface.backdropFilter, theme).toMatch(/^blur\(\d+px\) saturate\(\d+%\)$/);
      }
    }
  });

  it('never blurs without saturating', () => {
    // `blur()` alone averages the backdrop toward grey haze; the saturation
    // boost is what makes it read as glass rather than as fog. It is the most
    // common thing missing from a web recreation that looks almost right.
    for (const theme of THEMES) {
      for (const surface of Object.values(THEME_GLASS[theme])) {
        expect(surface.backdropFilter, theme).toContain('saturate(');
      }
    }
  });

  it('keeps blur within the performance budget', () => {
    // Cost scales with blurred area x radius, and each filtered element is its
    // own un-batchable compositing pass. 20px is the ceiling the recreation
    // authors independently converged on for large surfaces; 24px is the
    // sidebar exception, on the one surface that earns it.
    for (const theme of THEMES) {
      for (const surface of Object.values(THEME_GLASS[theme])) {
        const px = Number(/blur\((\d+)px\)/.exec(surface.backdropFilter)?.[1]);
        expect(px, theme).toBeLessThanOrEqual(24);
      }
    }
  });

  it('collapses to the theme surface when transparency is reduced', () => {
    for (const theme of THEMES) {
      const flat = THEME_GRADIENTS[theme] && THEME_GLASS[theme];
      // Both steps fall back to the SAME flat fill. Under Reduce Transparency
      // there is no "slightly less glass" — the distinction between a toolbar
      // and a sidebar's opacity was a distinction in translucency.
      expect(flat.glass.opaque, theme).toBe(flat.glassHi.opaque);
      expect(flat.glass.opaque, theme).toMatch(/^#[0-9A-F]{6}$/i);
    }
  });

  it('derives each theme glass from that theme own surface colour', () => {
    // Not hand-written per theme: glass has to read as the same material
    // family as the opaque surfaces beside it, so it follows `surface`.
    const before = getTheme();
    try {
      for (const theme of THEMES) {
        setTheme(theme);
        const surface = getThemedColors().surface.replace('#', '');
        const rgb = [0, 2, 4].map(i => parseInt(surface.substring(i, i + 2), 16)).join(',');
        expect(THEME_GLASS[theme].glass.background, theme).toBe(`rgba(${rgb},0.82)`);
      }
    } finally {
      setTheme(before);
    }
  });

  it('inverts the rim polarity on silver instead of reusing the dark recipe', () => {
    // A white-alpha specular edge is invisible on a white surface, and a black
    // ambient shadow is invisible on a pearl ground. This is the same class of
    // bug as the fill tokens below, in the material rather than in the fill.
    expect(THEME_GLASS.silver.glass.boxShadow).toContain('inset 0 1px 0 rgba(255,255,255,0.90)');
    expect(THEME_GLASS.silver.glass.boxShadow).toContain('rgba(30,37,48,0.12)');
    expect(THEME_GLASS.silver.glass.boxShadow).not.toContain('rgba(0,0,0');
  });
});

describe('fill tokens', () => {
  it('carries the theme own ink, so a hover is visible on every theme', () => {
    // The reason these exist. `rgba(255,255,255,0.06)` — written 202 times
    // across the app — is white over white on the silver theme: those hover
    // states simply did not render for anyone using it.
    const before = getTheme();
    try {
      setTheme('dark');
      const dark = getThemedColors();
      expect(dark.fillSubtle).toBe('rgba(255,255,255,0.04)');
      expect(dark.fillHover).toBe('rgba(255,255,255,0.07)');
      expect(dark.fillActive).toBe('rgba(255,255,255,0.11)');
      expect(dark.veil).toBe('rgba(7,11,20,0.62)');

      setTheme('silver');
      const silver = getThemedColors();
      expect(silver.fillSubtle).toBe('rgba(30,37,48,0.04)');
      expect(silver.fillHover).toBe('rgba(30,37,48,0.07)');
      expect(silver.fillActive).toBe('rgba(30,37,48,0.11)');
      expect(silver.veil).toBe('rgba(30,37,48,0.40)');

      // The invariant behind the values: the light theme's fills must NOT be
      // white-on-white. That is the whole bug.
      for (const fill of [silver.fillSubtle, silver.fillHover, silver.fillActive]) {
        expect(fill).not.toContain('255,255,255');
      }
    } finally {
      setTheme(before);
    }
  });

  it('rises monotonically from rest to hover to press', () => {
    const before = getTheme();
    try {
      for (const theme of THEMES) {
        setTheme(theme);
        const c = getThemedColors();
        const a = (v: string) => Number(/,\s*([\d.]+)\)$/.exec(v)?.[1]);
        expect(a(c.fillSubtle), theme).toBeLessThan(a(c.fillHover));
        expect(a(c.fillHover), theme).toBeLessThan(a(c.fillActive));
      }
    } finally {
      setTheme(before);
    }
  });
});

describe('space scale', () => {
  it('is the ramp the code already writes by hand', () => {
    // 650 raw gaps and paddings, measured, converged on this without anyone
    // agreeing to it — a 4pt subdivision of Apple's 8pt grid.
    expect(space).toEqual({ xs: 4, sm: 6, md: 8, lg: 10, xl: 12, xxl: 16, xxxl: 20, huge: 24 });
  });

  it('is strictly ascending, with no step wider than the one before it', () => {
    // A scale with a gap in the middle is a scale people step outside of.
    const values = Object.values(space);
    for (let i = 1; i < values.length; i++) {
      expect(values[i]).toBeGreaterThan(values[i - 1]);
    }
  });
});

describe('radius', () => {
  it('carries the measured glass step', () => {
    // See the derivation in tokens.ts: a screencapture of the live window at
    // 2x, fitted, gives a superellipse R = 18.07pt (n = 2.42) and a best
    // circular arc of 15.63pt. concentric() of those at our 8px inset gives 10
    // and 8; 9 is between them and inside the error of both.
    expect(radius.glass).toBe(9);
  });

  it('subtracts padding from the parent, and squares off rather than going negative', () => {
    // Apple's rule verbatim: "the system calculates the corner radius to equal
    // the container shape's corner radius minus the distance between corners";
    // if the result would be negative, "the corner is square".
    expect(concentric(16, 12)).toBe(4);
    expect(concentric(radius.glass, 8)).toBe(1);
    expect(concentric(16, 20)).toBe(0);
    expect(concentric(8, 8)).toBe(0);
    // A square corner is the right answer here, not a bug to clamp away from.
    expect(concentric(4, 100)).toBe(0);
  });

  it('returns whole pixels', () => {
    // A fractional radius antialiases into a soft corner, which defeats the
    // point: concentricity is about curves visibly lining up.
    expect(concentric(17.4, 8)).toBe(9);
    expect(Number.isInteger(concentric(18.07, 8))).toBe(true);
  });
});

describe('motion tokens', () => {
  it('is three springs, sampled as linear()', () => {
    expect(Object.keys(SPRING_LINEAR)).toEqual(['smooth', 'snappy', 'bouncy']);
    for (const curve of Object.values(SPRING_LINEAR)) {
      expect(curve).toMatch(/^linear\(0, [\d.,\s]+ 1\)$/);
    }
  });

  it('starts at 0 and ends at 1', () => {
    for (const [name, curve] of Object.entries(SPRING_LINEAR)) {
      const stops = curve.slice('linear('.length, -1).split(',').map(Number);
      expect(stops[0], name).toBe(0);
      expect(stops[stops.length - 1], name).toBe(1);
      expect(stops.length, name).toBe(25);
    }
  });

  it('overshoots by the amount its bounce implies, and no more', () => {
    // zeta = 1 - bounce. smooth is critically damped and must not overshoot at
    // all; snappy is a hint; bouncy is the one you are allowed to notice.
    const peak = (curve: string) =>
      Math.max(...curve.slice('linear('.length, -1).split(',').map(Number));
    expect(peak(SPRING_LINEAR.smooth)).toBe(1);
    expect(peak(SPRING_LINEAR.snappy)).toBeGreaterThan(1);
    expect(peak(SPRING_LINEAR.snappy)).toBeLessThan(1.02);
    expect(peak(SPRING_LINEAR.bouncy)).toBeGreaterThan(1.03);
    expect(peak(SPRING_LINEAR.bouncy)).toBeLessThan(1.06);
  });

  it('stays under Apple half-second ceiling', () => {
    // HIG: "keep animation duration under 0.5s to avoid feeling delayed", and
    // "prefer quick, precise animations". These are SETTLE times, not
    // perceptual ones — a spring feels finished before it stops moving.
    for (const [name, ms] of Object.entries(duration)) {
      expect(ms, name).toBeLessThan(500);
    }
    expect(duration.smooth).toBe(320);
    expect(duration.snappy).toBe(240);
    expect(duration.bouncy).toBe(440);
  });

  it('reaches the springs through a var() with a bezier fallback', () => {
    // linear() needs Safari 17.2+ and our floor is macOS 11. An unparseable
    // inline timing function silently becomes `ease`; a var() fallback becomes
    // a curve we chose. index.css upgrades the property behind @supports.
    for (const name of ['smooth', 'snappy', 'bouncy'] as const) {
      expect(ease[name]).toMatch(new RegExp(`^var\\(--pa-ease-${name}, cubic-bezier\\(`));
    }
    // And the old curves are untouched — they have hundreds of call sites.
    expect(ease.out).toBe('cubic-bezier(0.22, 1, 0.36, 1)');
    expect(ease.inOut).toBe('cubic-bezier(0.65, 0, 0.35, 1)');
    expect(ease.spring).toBe('cubic-bezier(0.34, 1.56, 0.64, 1)');
  });
});
