/**
 * Decision Inbox's own token-fitness gate (R7, the Apple glass-token pass
 * over the Decision Inbox — decision list, decision detail, answer/approve/
 * park controls, evidence digest, dead-letter line).
 *
 * Same shape as `styles/radiusScale.test.ts` / `styles/backdropFilter.test.ts`
 * / `styles/neonAccent.test.ts` / `automate/tokenFitness.test.ts`, scoped to
 * this directory. Those app-wide gates already cover their own ground; this
 * one is the directory-local promise that this screen specifically carries
 * zero hardcoded colors, radii or shadows going forward — every one of those
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
// in a comment — this directory references #302, #429, #458, #490, #503 and
// more — so it only counts as a color when it is actually quoted as a string
// value ('#fff', `#fff`), the only way a hex literal is ever written here.
const HEX_COLOR_3_QUOTED = /(?<=['"`])#[0-9a-fA-F]{3}(?=['"`])/g;

// The bug this whole lane exists to close: a theme token with a two-digit hex
// alpha suffix glued on by plain string concatenation — it only "works"
// because the token happens to be a bare `#rrggbb` today, and would silently
// produce garbage the moment it isn't. `withAlpha(colors.x, a)` (format.ts)
// is the replacement.
const HEX_ALPHA_SUFFIX = /colors\.\w+\s*\+\s*['"][0-9a-fA-F]{2}['"]/g;

describe('decisions: zero hardcoded colors', () => {
  it('names no hex color literal — every color comes from `colors.*` or a token', () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const text = readFileSync(file, 'utf8');
      const matches = [...(text.match(HEX_COLOR_6_8) ?? []), ...(text.match(HEX_COLOR_3_QUOTED) ?? [])];
      if (matches.length) offenders.push(`${file.slice(DIR.length)}: ${matches.join(', ')}`);
    }
    expect(offenders, 'use `colors.*` (or `withAlpha(colors.x, a)` for a tint) — not a hex literal').toEqual([]);
  });

  it('never derives a tint by string-concatenating a hex alpha suffix onto a token', () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const text = readFileSync(file, 'utf8');
      const matches = text.match(HEX_ALPHA_SUFFIX) ?? [];
      if (matches.length) offenders.push(`${file.slice(DIR.length)}: ${matches.join(', ')}`);
    }
    expect(offenders, "use withAlpha(colors.x, alpha) from './format' — not `colors.x + 'NN'`").toEqual([]);
  });
});

describe('decisions: zero hardcoded radii', () => {
  it('writes every corner radius from the `radius` scale, `concentric()`, or as a circle', () => {
    // '50%' (the checkmark circle, the tier-1 status dot) is a circle, not a
    // scale step, and is exempt by construction — this regex only matches a
    // bare number.
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
    expect(offenders, 'use radius.xs/sm/md/lg/xl/glass/pill or concentric(outer, padding) — not a raw number').toEqual([]);
  });
});

describe('decisions: zero hardcoded shadows', () => {
  it('never bakes a literal color into a boxShadow — every shadow color is a token', () => {
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
