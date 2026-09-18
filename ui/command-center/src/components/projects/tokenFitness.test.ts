/**
 * Projects' own token-fitness gate.
 *
 * Same shape as `styles/radiusScale.test.ts` / `styles/backdropFilter.test.ts`
 * / `styles/neonAccent.test.ts` and its sibling in `components/automate/`
 * (#1170), scoped to this directory (R12, the Apple glass pass over the
 * project board, the overview panels and the person drawer). Those app-wide
 * gates already cover their own ground; this one is the directory-local
 * promise that this screen specifically carries zero hardcoded colors, radii
 * or shadows going forward — a regression here fails CI on this file alone,
 * without waiting for a screen-wide sweep to notice.
 *
 * The fourth gate is this directory's own: `rgba()`. The panels here shared one
 * idiom — `theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)'`
 * — copied into eleven files before `fillSubtle`/`fillHover`/`fillActive`
 * existed to name it. Hex literals were never the problem in this directory;
 * that conditional was, so it is the one the gate has to hold shut.
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

function forEachLine(fn: (file: string, line: string, n: number) => void) {
  for (const file of FILES) {
    readFileSync(file, 'utf8').split('\n').forEach((line, i) => fn(file.slice(DIR.length), line, i + 1));
  }
}

// A literal `#RRGGBB`/`#RRGGBBAA` CSS color, six or eight hex digits — NOT an
// HTML numeric entity (`&#10003;`), which shares the `#digits` shape but is
// text, not a color (the negative lookbehind excludes it).
const HEX_COLOR_6_8 = /(?<!&)#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6})\b/g;

// The 3-digit shorthand (`#fff`) is the same shape as a GitHub issue number in
// a comment — this directory references #251, #490, #503, #530, #629, #1155 —
// so it only counts as a color when it is actually quoted as a string value,
// which is the only way a hex literal is ever written in this codebase.
const HEX_COLOR_3_QUOTED = /(?<=['"`])#[0-9a-fA-F]{3}(?=['"`])/g;

describe('projects: zero hardcoded colors', () => {
  it('names no hex color literal — every color comes from `colors.*` or a token', () => {
    const offenders: string[] = [];
    forEachLine((file, line, n) => {
      const hits = [...(line.match(HEX_COLOR_6_8) ?? []), ...(line.match(HEX_COLOR_3_QUOTED) ?? [])];
      if (hits.length) offenders.push(`${file}:${n}  ${hits.join(', ')}`);
    });
    expect(offenders, 'use `colors.*` — not a hex literal').toEqual([]);
  });

  it('writes no literal `rgba()` — the neutral fills are `fillSubtle`/`fillHover`/`fillActive`', () => {
    // The whole directory used to hand-roll the theme-conditional white/graphite
    // wash. `colors.fill*` carries the theme's OWN ink at the 4/7/11 ladder, so
    // one token reads as a lift on the void and as a shade on the pearl — which
    // is precisely what the hand-written version got wrong on silver.
    const RGBA_LITERAL = /rgba?\(\s*\d/;
    const offenders: string[] = [];
    forEachLine((file, line, n) => {
      if (RGBA_LITERAL.test(line)) offenders.push(`${file}:${n}  ${line.trim()}`);
    });
    expect(offenders, 'use colors.fillSubtle / fillHover / fillActive (or another `colors.*`)').toEqual([]);
  });
});

describe('projects: zero hardcoded radii', () => {
  it('writes every corner radius from the `radius` scale, `concentric()`, or as a circle', () => {
    // '50%' (a status dot, an avatar) is a circle, not a scale step, and is
    // exempt by construction — these regexes only match a bare number.
    const RAW_RADIUS = /borderRadius:\s*(-?\d+(?:\.\d+)?)\b/g;
    // The `Button` primitive takes its corner as a CSS custom property, which
    // is a string — so a raw radius hides there too, and did.
    const RAW_BTN_RADIUS = /'--pa-btn-radius':\s*'(-?\d+(?:\.\d+)?)px'/g;
    const offenders: string[] = [];
    forEachLine((file, line, n) => {
      for (const m of line.matchAll(RAW_RADIUS)) offenders.push(`${file}:${n}  ${m[1]}px`);
      for (const m of line.matchAll(RAW_BTN_RADIUS)) offenders.push(`${file}:${n}  --pa-btn-radius ${m[1]}px`);
    });
    expect(offenders, 'use radius.xs/sm/md/lg/xl/glass/pill or concentric(outer, padding)').toEqual([]);
  });
});

describe('projects: zero hardcoded shadows', () => {
  it('never bakes a literal color into a boxShadow — every shadow color is a token', () => {
    // A `boxShadow:` value may be `'none'`, or built from `${colors.x}` /
    // a `colors.elevation*` step — never a hex or a literal `rgba(<digit>…)`.
    const BOX_SHADOW = /boxShadow:\s*(`[^`]*`|'[^']*'|"[^"]*")/g;
    const LITERAL_COLOR = /#[0-9a-fA-F]{3,8}\b|rgba?\(\s*\d/;
    const offenders: string[] = [];
    forEachLine((file, line, n) => {
      for (const m of line.matchAll(BOX_SHADOW)) {
        if (LITERAL_COLOR.test(m[1])) offenders.push(`${file}:${n}  ${m[1]}`);
      }
    });
    expect(offenders, 'derive the shadow from colors.elevation* / colors.* — not a literal').toEqual([]);
  });
});

describe('projects: no glass in the content layer', () => {
  it('leaves `backdropFilter` to the `<Glass>` primitive', () => {
    // D1/D7: `styles/backdropFilter.test.ts` already owns this app-wide, but a
    // directory that just had a hand-rolled `blur(24px) saturate(140%)` drawer
    // in it keeps its own copy of the rule where the regression would happen.
    const offenders: string[] = [];
    forEachLine((file, line, n) => {
      if (/backdropFilter\s*:/.test(line)) offenders.push(`${file}:${n}  ${line.trim()}`);
    });
    expect(offenders, 'use <Glass> / useGlass() — and only on the floating control layer').toEqual([]);
  });
});
