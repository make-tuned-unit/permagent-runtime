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
});
