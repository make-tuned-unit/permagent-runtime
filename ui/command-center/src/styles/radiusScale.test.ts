/**
 * The radius scale, and the gate that keeps it honest.
 *
 * The scale used to be 6/10/14/20 while the app's most-used corner — 8px, by a
 * wide margin — was not in it, so 263 radii were written by hand. That is not
 * developers ignoring a scale; it is a scale that did not have the step they
 * needed. It has been rebased to 4/6/8/12/16, which is what they reached for.
 *
 * The second test is the part that matters going forward: once a value IS in
 * the scale, writing it as a bare number again is how the sprawl regrows.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { radius } from './tokens';

const SRC = fileURLToPath(new URL('..', import.meta.url));

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules') continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) { sourceFiles(full, out); continue; }
    if (!/\.tsx?$/.test(entry)) continue;
    if (/\.(test|spec)\.tsx?$/.test(entry)) continue;
    out.push(full);
  }
  return out;
}

describe('radius scale', () => {
  it('is the one the code converged on by hand', () => {
    // `glass` is the one step nobody converged on by hand, because nothing was
    // shaped like it yet: it is the outermost floating surface, derived from
    // the macOS window corner rather than chosen. Its derivation — and the
    // screencapture it was measured from — is in tokens.ts; the arithmetic is
    // re-checked in glassTokens.test.ts.
    expect(radius).toEqual({ xs: 4, sm: 6, md: 8, lg: 12, xl: 16, glass: 9, pill: 999 });
  });

  it('is not re-hardcoded anywhere the token already exists', () => {
    const inScale = new Set(Object.values(radius).map(String));
    const offenders: string[] = [];

    for (const file of sourceFiles(SRC)) {
      const lines = readFileSync(file, 'utf8').split('\n');
      lines.forEach((line, i) => {
        const m = /borderRadius: (\d+)\b/.exec(line);
        if (m && inScale.has(m[1])) offenders.push(`${file.slice(SRC.length)}:${i + 1}  ${m[1]}px`);
      });
    }

    expect(offenders, 'use the radius token — the scale has this value').toEqual([]);
  });
});
