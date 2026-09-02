/**
 * Automate's own token-fitness gate.
 *
 * Same shape as `styles/radiusScale.test.ts` / `styles/backdropFilter.test.ts`
 * / `styles/neonAccent.test.ts`, scoped to this directory (R10, the Apple
 * glass-token pass over Automate — schedules, recipes, storage insights).
 * Those app-wide gates already cover their own ground; this one is the
 * directory-local promise that this screen specifically carries zero
 * hardcoded colors, radii or shadows going forward — every one of those
 * regressing here fails CI on this file alone, without waiting for a
 * screen-wide sweep to notice.
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

// The 3-digit shorthand (`#fff`) is the same shape as a GitHub issue number
// in a comment — this file references #242, #193, #603 — so it only counts
// as a color when it is actually quoted as a string value ('#fff', `#fff`),
// which is the only way a hex literal is ever written in this codebase.
const HEX_COLOR_3_QUOTED = /(?<=['"`])#[0-9a-fA-F]{3}(?=['"`])/g;

describe('automate: zero hardcoded colors', () => {
  it('names no hex color literal — every color comes from `colors.*` or a token', () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const text = readFileSync(file, 'utf8');
      const matches = [...(text.match(HEX_COLOR_6_8) ?? []), ...(text.match(HEX_COLOR_3_QUOTED) ?? [])];
      if (matches.length) offenders.push(`${file.slice(DIR.length)}: ${matches.join(', ')}`);
    }
    expect(offenders, 'use `colors.*` (or `withAlpha(colors.x, a)` for a tint) — not a hex literal').toEqual([]);
  });
});

describe('automate: zero hardcoded radii', () => {
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

describe('automate: zero hardcoded shadows', () => {
  it('never bakes a literal color into a boxShadow — every shadow color is a token', () => {
    // A `boxShadow:` value is allowed to be `'none'`, or a template built from
    // `${colors.x}` / `${statusColor}` (itself always assigned from `colors.*`
    // in this directory) — never a hex or a literal `rgba(<digit>...)`.
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
