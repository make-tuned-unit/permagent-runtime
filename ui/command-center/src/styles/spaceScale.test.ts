/**
 * The spacing scale, and the two gates that keep it from being re-typed.
 *
 * Same protocol as `textScale.test.ts`: a size the scale owns is written as
 * `space.<step>`, never as the number; a size the scale does not own is
 * frozen per directory as a budget that may only go down.
 *
 * Scale (px): xxs=2, xs=4, sm=6, md=8, lg=10, xl=12, xxl=16, xxxl=20, huge=24.
 *
 * TO LOWER A BUDGET: change the number in the same commit that removes the
 * literals. It only ever moves down. Delete the entry when a directory hits 0.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

import { space } from './tokens';

const SRC = fileURLToPath(new URL('..', import.meta.url));
const TOKENS_FILE = 'styles/tokens.ts';

/**
 * Off-scale spacing literals still in the tree, by top-level directory.
 * Ceiling, not target — fails when a directory grows one; lowered by hand
 * when a directory sheds one. `browser` / `terminal` stay frozen for native
 * webview geometry (CHROME_GEOM) and are not migrated by AF1.
 */
const OFF_SCALE_BUDGET: Record<string, number> = {
  'components/automate': 9,
  'components/brain': 15,
  'components/build': 2,
  'components/common': 9,
  'components/dashboard': 27,
  'components/finance': 9,
  'components/grow': 6,
  'components/history': 1,
  'components/notifications': 5,
  'components/people': 2,
  'components/projects': 11,
  'components/settings': 12,
  'components/sidebar': 3,
  'components/splash': 1,
  'components/wizard': 21,
  'components/workspaces': 1,
  'components/world': 17,
};

const PROP =
  '(?:padding(?:Top|Right|Bottom|Left|Inline|Block)?|margin(?:Top|Right|Bottom|Left|Inline|Block)?|gap|rowGap|columnGap|top|left|right|bottom)';

const SCALE = new Set<number>(Object.values(space));

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry === 'harness') continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      // AF1 owns components/** and src/*.tsx only — skip other src trees
      // (hooks/, lib/, styles/ except this test's neighbours are not migrated here).
      const rel = relative(SRC, full);
      if (rel && !rel.startsWith('components') && rel !== '.') continue;
      sourceFiles(full, out);
      continue;
    }
    if (!/\.tsx?$/.test(entry)) continue;
    if (/\.(test|spec)\.tsx?$/.test(entry)) continue;
    // Root src/*.tsx are in scope; root src/*.ts (non-tsx) are not AF1's.
    const rel = relative(SRC, full);
    if (!rel.startsWith('components/') && !rel.endsWith('.tsx')) continue;
    if (rel.startsWith('styles/')) continue;
    out.push(full);
  }
  return out;
}

/** Spacing prop literals: `gap: 8`, `paddingTop: '12px'`, shorthand `'8px 12px'`. */
export function spacingLiterals(source: string): { line: number; px: number; prop: string }[] {
  const out: { line: number; px: number; prop: string }[] = [];
  const lines = source.split('\n');
  const single = new RegExp(
    `\\b(${PROP})(\\s*:\\s*)(?:(\\d+)(?=\\s*[,;}\\n]|\\s*//|\\s*$)|(['"\`])(\\d+)px\\4)`,
    'g',
  );
  const shorthand = new RegExp(`\\b(padding|margin)(\\s*:\\s*)(['"\`])([^'"\`]+)\\3`, 'g');

  lines.forEach((line, i) => {
    // Skip CHROME_GEOM and comment-only lines (prose can mention padding: '12px').
    if (line.includes('CHROME_GEOM')) return;
    const trimmed = line.trim();
    if (trimmed.startsWith('//') || trimmed.startsWith('*') || trimmed.startsWith('/*')) return;

    let m: RegExpExecArray | null;
    single.lastIndex = 0;
    while ((m = single.exec(line)) !== null) {
      const px = Number(m[3] ?? m[5]);
      if (!px) continue;
      out.push({ line: i + 1, px, prop: m[1] });
    }

    shorthand.lastIndex = 0;
    while ((m = shorthand.exec(line)) !== null) {
      const body = m[4];
      if (body.includes('${')) {
        // Mixed template: count only bare Npx parts still present.
        for (const part of body.trim().split(/\s+/)) {
          const mm = /^(\d+)px$/.exec(part);
          if (!mm) continue;
          const px = Number(mm[1]);
          if (!px) continue;
          out.push({ line: i + 1, px, prop: m[1] });
        }
        continue;
      }
      if (!/^\s*\d+px(?:\s+\d+px){0,3}\s*$/.test(body)) continue;
      for (const part of body.trim().split(/\s+/)) {
        const px = Number(part.replace('px', ''));
        if (!px) continue;
        out.push({ line: i + 1, px, prop: m[1] });
      }
    }
  });
  return out;
}

function dirKey(rel: string): string {
  if (rel.startsWith('components/')) {
    return rel.split('/').slice(0, 2).join('/');
  }
  // src/*.tsx → "src"
  return 'src';
}

function literalsByDir(): {
  onScale: Map<string, string[]>;
  offScale: Map<string, string[]>;
} {
  const onScale = new Map<string, string[]>();
  const offScale = new Map<string, string[]>();
  for (const file of sourceFiles(SRC)) {
    const rel = relative(SRC, file);
    if (rel === TOKENS_FILE) continue;
    if (rel.startsWith('harness/')) continue;
    const dir = dirKey(rel);
    // Native webview geometry — AF1 does not migrate these directories.
    if (dir === 'components/browser' || dir === 'components/terminal') continue;
    for (const { line, px, prop } of spacingLiterals(readFileSync(file, 'utf8'))) {
      const hit = `${rel}:${line}  ${prop}: ${px}`;
      if (SCALE.has(px)) {
        const list = onScale.get(dir) ?? [];
        list.push(hit);
        onScale.set(dir, list);
      } else {
        const list = offScale.get(dir) ?? [];
        list.push(hit);
        offScale.set(dir, list);
      }
    }
  }
  return { onScale, offScale };
}

describe('space scale', () => {
  it('exposes the steps the app already wrote, nothing else', () => {
    expect(space).toEqual({
      xxs: 2, xs: 4, sm: 6, md: 8, lg: 10, xl: 12, xxl: 16, xxxl: 20, huge: 24,
    });
  });

  it('is never re-typed as the number it already is', () => {
    const { onScale } = literalsByDir();
    const offenders: string[] = [];
    for (const [, hits] of onScale) offenders.push(...hits);
    expect(
      offenders,
      'use space.<step> from styles/tokens — the scale already has this size',
    ).toEqual([]);
  });

  it('grows no new size the scale does not have', () => {
    const { offScale } = literalsByDir();
    const over: string[] = [];
    for (const [dir, hits] of offScale) {
      const budget = OFF_SCALE_BUDGET[dir];
      if (budget === undefined) {
        over.push(`${dir}: ${hits.length} off-scale sizes, and no budget declared\n    ${hits.slice(0, 8).join('\n    ')}`);
      } else if (hits.length > budget) {
        over.push(`${dir}: declares ${budget}, found ${hits.length}\n    ${hits.join('\n    ')}`);
      }
    }
    expect(
      over,
      'off-scale spacing is frozen: migrate to space.*, or say why the budget must rise',
    ).toEqual([]);
  });

  it('declares no budget for a directory that has none left', () => {
    const { offScale } = literalsByDir();
    const stale = Object.keys(OFF_SCALE_BUDGET).filter(d => !offScale.has(d));
    expect(stale, 'this directory is clean — delete its budget line').toEqual([]);
  });
});
