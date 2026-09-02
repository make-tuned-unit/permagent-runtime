/**
 * Grow's own token-fitness gate.
 *
 * Same shape as `styles/radiusScale.test.ts` / `styles/backdropFilter.test.ts`
 * / `styles/neonAccent.test.ts` and as `automate/tokenFitness.test.ts`, scoped
 * to this directory (R9, the Apple glass-token pass over Grow — actions,
 * results, strategy, calendar, analytics). Those app-wide gates already cover
 * their own ground; this one is the directory-local promise that this screen
 * specifically carries zero hardcoded colors, radii or shadows going forward — every one of those regressing here fails CI on this
 * file alone, without waiting for a screen-wide sweep to notice.
 *
 * TYPE IS NOT HERE ON PURPOSE. `styles/textScale.test.ts` already owns the ramp
 * app-wide with a per-directory budget, and R9 took `components/grow` from 52
 * off-ramp sizes to 0 in the same commit that lowered its entry — which is the
 * protocol that file documents. A second gate for the same rule is the "one
 * concept, one place" failure this pass exists to remove.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const DIR = fileURLToPath(new URL('.', import.meta.url));

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) { sourceFiles(full, out); continue; }
    if (!/\.tsx?$/.test(entry)) continue;
    if (/\.(test|spec)\.tsx?$/.test(entry)) continue;
    out.push(full);
  }
  return out;
}

const FILES = sourceFiles(DIR);

// A literal `#RRGGBB`/`#RRGGBBAA` CSS color, six or eight hex digits — NOT an
// HTML numeric entity (`&#10003;`, `&#8226;`), which shares the `#digits`
// shape but is text, not a color (the negative lookbehind excludes it).
const HEX_COLOR_6_8 = /(?<!&)#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6})\b/g;

// The 3-digit shorthand (`#fff`) is the same shape as a GitHub issue number in
// a comment — this directory references #22, #23, #1053, #1167 — so it only
// counts as a color when it is actually quoted as a string value ('#fff'),
// which is the only way a hex literal is ever written in this codebase.
const HEX_COLOR_3_QUOTED = /(?<=['"`])#[0-9a-fA-F]{3}(?=['"`])/g;

describe('grow: zero hardcoded colors', () => {
  it('names no hex color literal — every color comes from `colors.*` or a token', () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const text = readFileSync(file, 'utf8');
      const matches = [...(text.match(HEX_COLOR_6_8) ?? []), ...(text.match(HEX_COLOR_3_QUOTED) ?? [])];
      if (matches.length) offenders.push(`${file.slice(DIR.length)}: ${matches.join(', ')}`);
    }
    expect(offenders, 'use `colors.*` (or a tint of one) — not a hex literal').toEqual([]);
  });

  // `rgba(0,0,0,0.3)` is invisible-or-wrong on the silver theme in exactly the
  // way `rgba(255,255,255,0.06)` is: a fixed ink over a surface whose polarity
  // flips. The theme's own `fillSubtle`/`fillHover`/`fillActive`/`veil` carry
  // the right ink per theme, which is the whole reason they exist.
  it('writes no literal rgb()/rgba() — the theme owns its ink', () => {
    const LITERAL_RGB = /\brgba?\(\s*\d/g;
    const offenders: string[] = [];
    for (const file of FILES) {
      const lines = readFileSync(file, 'utf8').split('\n');
      lines.forEach((line, i) => {
        for (const m of line.matchAll(LITERAL_RGB)) {
          offenders.push(`${file.slice(DIR.length)}:${i + 1}  ${m[0]}…`);
        }
      });
    }
    expect(offenders, 'use colors.fillSubtle / fillHover / fillActive / veil — not a literal rgba').toEqual([]);
  });
});

describe('grow: zero hardcoded radii', () => {
  it('writes every corner radius from the `radius` scale, `concentric()`, or as a circle', () => {
    // '50%' (a status dot, an avatar) is a circle, not a scale step, and is
    // exempt by construction — this regex only matches a bare number.
    const RAW_RADIUS = /borderRadius:\s*(-?\d+(?:\.\d+)?)\b/g;
    const offenders: string[] = [];
    for (const file of FILES) {
      const lines = readFileSync(file, 'utf8').split('\n');
      lines.forEach((line, i) => {
        for (const m of line.matchAll(RAW_RADIUS)) {
          offenders.push(`${file.slice(DIR.length)}:${i + 1}  ${m[1]}px`);
        }
      });
    }
    expect(offenders, 'use radius.xs/sm/md/lg/xl/glass or concentric(outer, padding) — not a raw number').toEqual([]);
  });
});

describe('grow: zero hardcoded shadows', () => {
  it('never bakes a literal color into a boxShadow — every shadow color is a token', () => {
    // A `boxShadow:` value is allowed to be `'none'`, or a template built from
    // `${colors.x}` — never a hex or a literal `rgba(<digit>...)`.
    const BOX_SHADOW_LINE = /boxShadow:\s*(`[^`]*`|'[^']*'|"[^"]*")/g;
    const LITERAL_COLOR = /#[0-9a-fA-F]{3,8}\b|rgba?\(\s*\d/;
    const offenders: string[] = [];
    for (const file of FILES) {
      const lines = readFileSync(file, 'utf8').split('\n');
      lines.forEach((line, i) => {
        for (const m of line.matchAll(BOX_SHADOW_LINE)) {
          if (LITERAL_COLOR.test(m[1])) offenders.push(`${file.slice(DIR.length)}:${i + 1}  ${m[1]}`);
        }
      });
    }
    expect(offenders, 'derive the shadow color from `colors.*` — not a literal hex/rgb').toEqual([]);
  });
});

describe('grow: glass stays on the floating layer', () => {
  // D1/D7, and the reason `styles/backdropFilter.test.ts` exists app-wide:
  // `common/Glass.tsx` is the sole owner of the property. A card, a list row or
  // a panel body reaching for its own blur is the #1 anti-slop tell, and it is
  // also a compositing pass per surface.
  it('hand-writes no backdropFilter — glass comes from <Glass>/glassSurface()', () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const lines = readFileSync(file, 'utf8').split('\n');
      lines.forEach((line, i) => {
        if (/backdropFilter\s*:/.test(line)) offenders.push(`${file.slice(DIR.length)}:${i + 1}`);
      });
    }
    expect(offenders, 'content is opaque; floating controls use <Glass> or glassSurface()').toEqual([]);
  });
});
