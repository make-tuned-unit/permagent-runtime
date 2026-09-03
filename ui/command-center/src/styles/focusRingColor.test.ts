// AF2a: the app-wide `:focus-visible` ring (index.css ~L396) matched its
// selector but never painted — `getComputedStyle(el).outlineStyle` read
// `'none'` even with `el.matches(':focus-visible') === true` (#1178,
// reported on toast buttons; reproduced live in Chromium, confirmed global).
//
// Root cause: `--tw-accent` (and its siblings synced by tokens.ts
// `_syncCssVars`) is stored as a bare "R G B" channel triplet, e.g.
// "0 213 255" — never a standalone `<color>` — so Tailwind's alpha-modifier
// utilities can wrap it as `rgb(var(--tw-accent) / <alpha-value>)`
// (tailwind.config.js). `index.css` used it unwrapped, as
// `var(--tw-accent, #00D5FF)`, directly as an `outline` color. A bare
// triplet isn't a valid `<color>`, so the whole `outline` declaration was
// invalid at computed-value time — and because `--tw-accent` genuinely *is*
// defined (just invalid in this context), the `#00D5FF` fallback never
// fired either: a `var()` fallback only applies when the custom property is
// unset. Net effect: outline silently computed to `none` app-wide.
//
// This guards the fix (wrap in `rgb(...)`, the same pattern
// tailwind.config.js already uses) and, gate-1-style, keeps every triplet
// custom property from regrowing a bare, unwrapped `var(--tw-…)` use
// anywhere in src — jsdom's CSSOM doesn't model computed-value-time
// invalidity, so this is a static text gate rather than a computed-style
// assertion (jsdom happily "parses" the invalid form, which is exactly why
// this shipped unnoticed).
import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC_DIR = join(dirname(fileURLToPath(import.meta.url)), '..');
const THIS_FILE = fileURLToPath(import.meta.url);
const SOURCE_EXT = /\.(ts|tsx|css)$/;

// Strip /* ... */ comments before scanning: doc comments are allowed to name
// the bare-var anti-pattern as an example (as this file's own header does),
// and that must not itself trip the gate.
function stripBlockComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, '');
}

// Every custom property tokens.ts `_syncCssVars` stores as a bare "R G B"
// triplet (via `_hex()`) rather than a full color — the ones that are only
// valid wrapped in `rgb(...)`. `--tw-accent-glow` and friends are excluded:
// they carry full rgba() strings and are valid bare.
const TRIPLET_VARS = [
  'tw-dark-bg', 'tw-dark-surface', 'tw-dark-surface-2', 'tw-dark-text', 'tw-dark-muted',
  'tw-accent', 'tw-accent-dim', 'tw-input-bg', 'tw-danger', 'tw-success', 'tw-warning',
  'tw-status-ok', 'tw-status-warn', 'tw-status-error', 'tw-status-info',
];

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (SOURCE_EXT.test(name)) out.push(p);
  }
  return out;
}

describe('focus ring: --tw-accent triplet must resolve through rgb()', () => {
  it('index.css :focus-visible rules wrap --tw-accent in rgb()', () => {
    const css = readFileSync(join(SRC_DIR, 'index.css'), 'utf8');
    const focusBlockRe = /:focus-visible[^}]*\{[^}]*\}/g;
    const blocks = css.match(focusBlockRe) ?? [];
    expect(blocks.length).toBeGreaterThan(0);
    for (const block of blocks) {
      if (!block.includes('--tw-accent')) continue;
      // Every reference to the triplet var must be inside an rgb(...) call.
      expect(block).toMatch(/rgb\(\s*var\(--tw-accent\b/);
      // And must not appear bare (outline: … solid var(--tw-accent…)).
      expect(block).not.toMatch(/solid\s+var\(--tw-accent\b/);
    }
  });

  it('no triplet-only custom property is used unwrapped by rgb()/rgba() anywhere in src', () => {
    const offenders: string[] = [];
    for (const file of walk(SRC_DIR)) {
      if (file === THIS_FILE) continue;
      const text = stripBlockComments(readFileSync(file, 'utf8'));
      for (const name of TRIPLET_VARS) {
        // Every occurrence of `var(--name` in the file, with the 6 chars
        // immediately before it (to check for an `rgb(`/`rgba(` wrapper,
        // allowing a little whitespace).
        const re = new RegExp(`(.{0,8})var\\(--${name}(?![\\w-])`, 'g');
        let m: RegExpExecArray | null;
        while ((m = re.exec(text))) {
          const before = m[1];
          if (!/rgba?\(\s*$/.test(before)) {
            offenders.push(`${file}: --${name} used unwrapped ("...${before}var(--${name}...")`);
          }
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});
