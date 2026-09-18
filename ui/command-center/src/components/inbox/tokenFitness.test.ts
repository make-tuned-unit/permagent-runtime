/**
 * Inbox's own token-fitness gate.
 *
 * Same shape as `automate/tokenFitness.test.ts` / `styles/radiusScale.test.ts`,
 * scoped to this directory (R17, the Apple glass-token pass over the Downloads
 * inbox). App-wide gates already cover their own ground; this one is the
 * directory-local promise that this screen specifically carries zero
 * hardcoded colors, radii or shadows going forward.
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

const HEX_COLOR_6_8 = /(?<!&)#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6})\b/g;
const HEX_COLOR_3_QUOTED = /(?<=['"`])#[0-9a-fA-F]{3}(?=['"`])/g;

describe('inbox: zero hardcoded colors', () => {
  it('names no hex color literal — every color comes from `colors.*` or a token', () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const text = readFileSync(file, 'utf8');
      const matches = [...(text.match(HEX_COLOR_6_8) ?? []), ...(text.match(HEX_COLOR_3_QUOTED) ?? [])];
      if (matches.length) offenders.push(`${file.slice(DIR.length)}: ${matches.join(', ')}`);
    }
    expect(offenders, 'use `colors.*` — not a hex literal').toEqual([]);
  });
});

describe('inbox: zero hardcoded radii', () => {
  it('writes every corner radius from the `radius` scale, `concentric()`, or as a circle', () => {
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
    expect(offenders, 'use radius.* or concentric(outer, padding) — not a raw number').toEqual([]);
  });
});

describe('inbox: zero hardcoded shadows', () => {
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

describe('inbox: zero native title= tooltips', () => {
  it('has no raw title= left — every tip goes through the Tooltip primitive', () => {
    const TITLE_ATTR = /\btitle\s*=/;
    const offenders: string[] = [];
    for (const file of FILES) {
      if (!file.endsWith('.tsx')) continue;
      const lines = readFileSync(file, 'utf8').split('\n');
      lines.forEach((line, i) => {
        const trimmed = line.trim();
        if (trimmed.startsWith('//') || trimmed.startsWith('*') || trimmed.startsWith('/*')) return;
        if (TITLE_ATTR.test(line)) offenders.push(`${file.slice(DIR.length)}:${i + 1}`);
      });
    }
    expect(offenders, 'wrap with <Tooltip content=…>; do not leave native title=').toEqual([]);
  });
});
