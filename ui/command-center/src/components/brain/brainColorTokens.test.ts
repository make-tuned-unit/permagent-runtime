/**
 * Zero hardcoded colors on the Brain screen (R13 Liquid Glass pass, DAG rule
 * gate 1). Follows the shape of `styles/neonAccent.test.ts` and
 * `styles/backdropFilter.test.ts`: walk the source, flag the literal forms
 * a hand-written color takes, and name the (small, documented) set of files
 * that get a pass and why — rather than silently skipping them.
 *
 * What counts as "hardcoded": a `#rrggbb`/`#rgb` literal or an `rgba()`/
 * `rgb()` call written directly in a `.tsx`/`.ts` file, outside a comment.
 * Every DOM-facing color on this screen should instead come from
 * `useTheme().colors`, the `THEME_GLASS` tokens (via `<Glass>` /
 * `glassSurface()`), or a token-derived expression (`` `${colors.cyan}1f` ``
 * — a real string, but built from a theme value, not a magic number).
 *
 * What is exempt, and why:
 *
 *   - `BrainScene.ts` and `graphPalette.ts` are the 3D graph's own palette —
 *     WebGL materials and a canvas-2D label texture, which take real color
 *     strings/numbers by construction (a `<canvas>` fillStyle or a Three.js
 *     `MeshBasicMaterial` cannot consume a CSS custom property). This is the
 *     same call the app already made for `components/world/constants.ts` in
 *     `neonAccent.test.ts` — a parallel, self-consistent rendering system,
 *     not DOM chrome. `graphPalette.ts`'s numbering is `0xRRGGBB` (Three.js
 *     color ints), which this test's regexes don't even match; `BrainScene.ts`
 *     carries a handful of real CSS-form literals (canvas fillStyle/shadow for
 *     node-label sprites) that are exempted explicitly below.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('.', import.meta.url));
const THIS_FILE = fileURLToPath(import.meta.url);

/** The 3D scene's own rendering code — see file header. */
const EXEMPT = new Set(['BrainScene.ts', 'graphPalette.ts']);

const HEX = /#[0-9a-fA-F]{3,8}\b/g;
const RGB_FN = /\brgba?\(/g;

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules') continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) { sourceFiles(full, out); continue; }
    if (!/\.tsx?$/.test(entry)) continue;
    out.push(full);
  }
  return out;
}

/** Strip `/* … *‍/` and `// …` comments before scanning, so an issue number
 *  in prose (`#587-adjacent`) or a doc example never reads as a color. */
function stripComments(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1');
}

function scan(): { file: string; hex: number; rgb: number }[] {
  const hits: { file: string; hex: number; rgb: number }[] = [];
  for (const file of sourceFiles(SRC)) {
    if (file === THIS_FILE) continue;
    const rel = file.slice(SRC.length).split(sep).join('/');
    if (EXEMPT.has(rel)) continue;
    if (/\.(test|spec)\.tsx?$/.test(rel)) continue;
    const code = stripComments(readFileSync(file, 'utf8'));
    const hex = (code.match(HEX) ?? []).length;
    const rgb = (code.match(RGB_FN) ?? []).length;
    if (hex > 0 || rgb > 0) hits.push({ file: rel, hex, rgb });
  }
  return hits;
}

describe('Brain screen: zero hardcoded colors (R13)', () => {
  it('writes no literal #hex or rgba()/rgb() color outside the 3D scene palette', () => {
    const offenders = scan();
    expect(
      offenders,
      'use useTheme().colors, the glass tokens (<Glass>/glassSurface()), or a '
      + 'token-derived expression — not a literal hex/rgba color. If a value is '
      + 'genuinely a 3D-scene palette entry, add it to EXEMPT with a reason, the '
      + 'way BrainScene.ts and graphPalette.ts already are.',
    ).toEqual([]);
  });

  it('names its own exemptions so the list cannot silently grow', () => {
    // Guards the guard: if EXEMPT gained an entry, this test would need
    // updating too, which is the point — an unreviewed addition is visible.
    expect([...EXEMPT].sort()).toEqual(['BrainScene.ts', 'graphPalette.ts']);
  });
});
