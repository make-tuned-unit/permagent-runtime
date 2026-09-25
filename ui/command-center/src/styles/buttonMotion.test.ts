// AF2a D9: `.pa-btn` (index.css, the Button primitive's shared rule set) used
// to transition on a hand-copied `cubic-bezier(0.22, 1, 0.36, 1)` literal —
// numerically identical to `--pa-ease-smooth`'s own bezier fallback, which is
// exactly the trap: hardcoding the fallback opted every button in the app out
// of the `@supports (transition-timing-function: linear(0, 1))` upgrade that
// swaps `--pa-ease-*` for the real sampled spring (SPRING_LINEAR in
// tokens.ts) on anything current. This pins the fix (reference the CSS
// custom property, not a literal) and keeps every `.pa-btn` transition under
// D9's 500ms ceiling.
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const CSS_PATH = join(dirname(fileURLToPath(import.meta.url)), '..', 'index.css');

function pabtnTransitionBlock(css: string): string {
  const start = css.indexOf('.pa-btn {');
  expect(start).toBeGreaterThan(-1);
  const end = css.indexOf('}', start);
  return css.slice(start, end);
}

describe('.pa-btn motion (D9)', () => {
  const css = readFileSync(CSS_PATH, 'utf8');
  const block = pabtnTransitionBlock(css);

  it('uses the --pa-ease-* spring tokens, not a hardcoded cubic-bezier literal', () => {
    expect(block).toMatch(/transition:/);
    expect(block).not.toMatch(/cubic-bezier/);
    // Every transitioned property names a --pa-ease-* var as its timing function.
    const props = ['background-color', 'border-color', 'color', 'box-shadow', 'opacity', 'transform'];
    for (const prop of props) {
      const re = new RegExp(`${prop}\\s+\\d+ms\\s+var\\(--pa-ease-\\w+\\)`);
      expect(block).toMatch(re);
    }
  });

  it('every transitioned property stays under D9\'s 500ms ceiling', () => {
    const durations = [...block.matchAll(/(\d+)ms\s+var\(--pa-ease-/g)].map(m => Number(m[1]));
    expect(durations.length).toBeGreaterThan(0);
    for (const ms of durations) expect(ms).toBeLessThan(500);
  });

  it('Reduce Motion still collapses it: the app-wide guard uses !important on transition-duration', () => {
    const guardStart = css.indexOf('@media (prefers-reduced-motion: reduce)');
    expect(guardStart).toBeGreaterThan(-1);
    const guardBlock = css.slice(guardStart, guardStart + 400);
    expect(guardBlock).toMatch(/transition-duration:\s*0\.001ms\s*!important/);
    // Applies universally, so it reaches `.pa-btn` buttons with no per-class opt-out needed.
    expect(guardBlock).toMatch(/\*\s*,/);
  });

  it('the press-in overrides transition-duration so the dip reads as near-instant', () => {
    const activeStart = css.indexOf('.pa-btn:active:not(:disabled) {');
    expect(activeStart).toBeGreaterThan(-1);
    const activeBlock = css.slice(activeStart, css.indexOf('}', activeStart));
    expect(activeBlock).toMatch(/transform:\s*scale\(0\.96\)/);
    expect(activeBlock).toMatch(/transition-duration:\s*60ms/);
  });
});

describe('failure state (R8): shake + danger tint on a rejected promise', () => {
  const css = readFileSync(CSS_PATH, 'utf8');

  it('animates a bounded, single-pass shake', () => {
    expect(css).toMatch(/\.pa-btn\[data-state='failure'\]\s*{[^}]*animation:\s*pa-btn-shake\s+\d+ms/);
    const kfStart = css.indexOf('@keyframes pa-btn-shake');
    expect(kfStart).toBeGreaterThan(-1);
  });

  it('stays under D9\'s 500ms ceiling', () => {
    const match = css.match(/\.pa-btn\[data-state='failure'\][^}]*animation:\s*pa-btn-shake\s+(\d+)ms/);
    expect(match).toBeTruthy();
    expect(Number(match![1])).toBeLessThan(500);
  });
});

describe('global pressable feedback (R8): native roles + .pa-pressable', () => {
  const css = readFileSync(CSS_PATH, 'utf8');

  it('covers every role the brief names, plus the escape-hatch class', () => {
    const selectorsBlockStart = css.indexOf("button,\n[role='button']");
    expect(selectorsBlockStart).toBeGreaterThan(-1);
    const block = css.slice(selectorsBlockStart, css.indexOf('{', selectorsBlockStart));
    for (const sel of ["[role='button']", "[role='tab']", "[role='menuitem']", "[role='option']", 'summary', "a[href]", '.pa-pressable']) {
      expect(block).toContain(sel);
    }
  });

  it('.pa-pressable presses with transform/filter only (compositor-friendly, no shadow/glow)', () => {
    // The standalone `.pa-pressable { cursor: pointer; }` rule (as opposed to
    // its appearance inside the shared multi-selector list above it).
    const start = css.indexOf('.pa-pressable {\n  cursor: pointer;');
    expect(start).toBeGreaterThan(-1);
    const block = css.slice(start, start + 400);
    expect(block).not.toMatch(/box-shadow/);
    expect(block).toMatch(/filter:\s*brightness/);
  });
});

describe('.pa-enter (R8): fade + rise, staggerable, capped duration', () => {
  const css = readFileSync(CSS_PATH, 'utf8');

  it('animates opacity and a small translateY only', () => {
    const kf = css.slice(css.indexOf('@keyframes pa-enter'), css.indexOf('@keyframes pa-enter') + 200);
    expect(kf).toMatch(/opacity:\s*0/);
    expect(kf).toMatch(/translateY\(7px\)/);
  });

  it('exposes a per-item delay custom property for staggering', () => {
    const start = css.indexOf('.pa-enter {');
    const block = css.slice(start, css.indexOf('}', start));
    expect(block).toMatch(/animation-delay:\s*var\(--pa-enter-delay/);
  });

  it('stays under the 500ms ceiling', () => {
    const start = css.indexOf('.pa-enter {');
    const block = css.slice(start, css.indexOf('}', start));
    const match = block.match(/animation:\s*pa-enter\s+(\d+)ms/);
    expect(match).toBeTruthy();
    expect(Number(match![1])).toBeLessThan(500);
  });
});

describe('skeleton shimmer, modal/popover entrance, agent pulse, toast springs', () => {
  const css = readFileSync(CSS_PATH, 'utf8');

  it('.pa-skeleton sweeps a background-position, not layout-affecting properties', () => {
    expect(css).toContain('@keyframes pa-skeleton-sweep');
    expect(css).toMatch(/\.pa-skeleton\s*{[^}]*animation:\s*pa-skeleton-sweep/);
  });

  it('.pa-modal-in and .pa-popover-in scale from a fraction toward 1 (scale-from-origin)', () => {
    expect(css).toMatch(/@keyframes pa-modal-in\s*{\s*from\s*{\s*opacity:\s*0;\s*transform:\s*scale\(0\.96\)/);
    expect(css).toMatch(/@keyframes pa-popover-in\s*{\s*from\s*{\s*opacity:\s*0;\s*transform:\s*scale\(0\.94\)/);
  });

  it('.pa-agent-pulse animates opacity only (never a data value)', () => {
    const kf = css.slice(css.indexOf('@keyframes pa-agent-pulse'), css.indexOf('@keyframes pa-agent-pulse') + 150);
    expect(kf).toMatch(/opacity:\s*1/);
    expect(kf).not.toMatch(/transform/);
  });

  it('toast enter is a spring, exit is quicker than enter', () => {
    const inMatch = css.match(/\.pa-toast-in\s*{\s*animation:\s*pa-toast-in\s+(\d+)ms/);
    const outMatch = css.match(/\.pa-toast-out\s*{\s*animation:\s*pa-toast-out\s+(\d+)ms/);
    expect(inMatch).toBeTruthy();
    expect(outMatch).toBeTruthy();
    expect(Number(outMatch![1])).toBeLessThan(Number(inMatch![1]));
  });
});

describe('the app\'s own Reduce Motion setting collapses motion too, not only the OS one (R8)', () => {
  const css = readFileSync(CSS_PATH, 'utf8');

  it('mirrors the same !important collapse under [data-reduce-motion="true"]', () => {
    const start = css.indexOf(":root[data-reduce-motion='true']");
    expect(start).toBeGreaterThan(-1);
    const block = css.slice(start, start + 400);
    expect(block).toMatch(/animation-duration:\s*0\.001ms\s*!important/);
    expect(block).toMatch(/transition-duration:\s*0\.001ms\s*!important/);
  });
});
