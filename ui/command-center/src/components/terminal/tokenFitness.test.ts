/**
 * Terminal's own token-fitness gate.
 *
 * Same shape as `automate/tokenFitness.test.ts` / `brain/brainColorTokens.test.ts`,
 * scoped to this directory (R15, Apple glass-token pass over terminal chrome).
 * The ANSI palette in `xtermTheme.ts` is the PTY content renderer’s own
 * curated set — no DOM chrome — so it is named and frozen as an exemption,
 * the same way Brain exempts its WebGL graph palette.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const DIR = fileURLToPath(new URL('.', import.meta.url));
const THIS_FILE = fileURLToPath(import.meta.url);

/** PTY content ANSI palette — see file header. Not chrome. */
const EXEMPT = new Set(['xtermTheme.ts']);

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

function stripComments(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1');
}

const FILES = sourceFiles(DIR).filter((f) => f !== THIS_FILE);

const HEX_COLOR_6_8 = /(?<!&)#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6})\b/g;
const HEX_COLOR_3_QUOTED = /(?<=['"`])#[0-9a-fA-F]{3}(?=['"`])/g;

describe('terminal: zero hardcoded colors (chrome)', () => {
  it('names no hex color literal outside the xterm ANSI palette', () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const rel = file.slice(DIR.length).split(sep).join('/');
      if (EXEMPT.has(rel)) continue;
      const text = stripComments(readFileSync(file, 'utf8'));
      const matches = [...(text.match(HEX_COLOR_6_8) ?? []), ...(text.match(HEX_COLOR_3_QUOTED) ?? [])];
      if (matches.length) offenders.push(`${rel}: ${matches.join(', ')}`);
    }
    expect(offenders, 'use `colors.*` — not a hex literal').toEqual([]);
  });

  it('names its own exemptions so the list cannot silently grow', () => {
    expect([...EXEMPT].sort()).toEqual(['xtermTheme.ts']);
  });
});

describe('terminal: zero hardcoded radii', () => {
  it('writes every corner radius from the `radius` scale, `concentric()`, or CHROME helpers', () => {
    const RAW_RADIUS = /borderRadius:\s*(-?\d+(?:\.\d+)?)\b/g;
    const offenders: string[] = [];
    for (const file of FILES) {
      const rel = file.slice(DIR.length).split(sep).join('/');
      if (EXEMPT.has(rel)) continue;
      const lines = stripComments(readFileSync(file, 'utf8')).split('\n');
      lines.forEach((line, i) => {
        for (const m of line.matchAll(RAW_RADIUS)) {
          offenders.push(`${rel}:${i + 1}  ${m[1]}px`);
        }
      });
    }
    expect(offenders, 'use radius.* / concentric() / CHROME_RADIUS — not a raw number').toEqual([]);
  });
});

describe('terminal: zero hardcoded shadows', () => {
  it('never bakes a literal color into a boxShadow', () => {
    const BOX_SHADOW_LINE = /boxShadow:\s*(`[^`]*`|'[^']*'|"[^"]*")/g;
    const LITERAL_COLOR = /#[0-9a-fA-F]{3,8}\b|rgba?\(\s*\d/;
    const offenders: string[] = [];
    for (const file of FILES) {
      const rel = file.slice(DIR.length).split(sep).join('/');
      if (EXEMPT.has(rel)) continue;
      const lines = stripComments(readFileSync(file, 'utf8')).split('\n');
      lines.forEach((line, i) => {
        for (const m of line.matchAll(BOX_SHADOW_LINE)) {
          if (LITERAL_COLOR.test(m[1])) offenders.push(`${rel}:${i + 1}  ${m[1]}`);
        }
      });
    }
    expect(offenders, 'derive the shadow from `colors.*` — not a literal hex/rgb').toEqual([]);
  });
});

describe('terminal: no content-layer backdropFilter', () => {
  it('never names backdropFilter outside useGlass (D7)', () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const rel = file.slice(DIR.length).split(sep).join('/');
      const text = stripComments(readFileSync(file, 'utf8'));
      if (/backdropFilter\s*:|WebkitBackdropFilter\s*:|backdrop-filter\s*:/.test(text)) {
        offenders.push(rel);
      }
    }
    expect(offenders).toEqual([]);
  });
});
