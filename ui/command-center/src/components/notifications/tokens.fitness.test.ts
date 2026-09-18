/**
 * Fitness gate for this directory (UI DAG rule 1): zero hardcoded colors,
 * radii, or shadows on the notification tray/toast screen.
 *
 * The repo already runs three gates of this shape app-wide — `radiusScale
 * .test.ts` (no re-hardcoded radius), `backdropFilter.test.ts` (glass only
 * from `styles/tokens.ts` / `components/common/Glass.tsx`), `neonAccent
 * .test.ts` (the one canonical cyan) — so this directory is already covered
 * by those for the cases they check. What is missing, and what none of them
 * catch, is a literal hex color, an `rgba(...)` literal, or a hand-rolled
 * `boxShadow:` string written directly in `components/notifications/*`
 * instead of coming from `useTheme().colors` / the glass token. This test is
 * that gate, scoped to this lane's own files, so it can be evidence in this
 * PR rather than relying on someone reading the diff by eye.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const DIR = fileURLToPath(new URL('.', import.meta.url));
const THIS_FILE = fileURLToPath(import.meta.url);

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

/** A hex color literal used as a value — `'#fff'`, `"#00d5ff"`, `` `#00d5ffcc` ``.
 *  Requires the leading quote so an issue reference in prose (`#618`) doesn't
 *  false-positive: a real color is always a string/template value, never bare
 *  in a comment. */
const HEX_COLOR = /['"`]#[0-9a-f]{3,8}\b/gi;
/** An `rgb(...)`/`rgba(...)` literal used as a value. Glass tokens and theme
 *  colors already carry rgba strings — this directory should only ever
 *  reference them by name (`colors.borderHi`, `glass.background`, …), never
 *  spell one out itself. */
const RGBA_LITERAL = /rgba?\(\s*\d/gi;
/** A hand-rolled `boxShadow:` value — as opposed to `boxShadow: glass
 *  .boxShadow` or `boxShadow: \`${glass.boxShadow}, ${colors.elevationFloating}\``,
 *  which reference tokens rather than spelling out a shadow. */
const HARDCODED_BOX_SHADOW = /boxShadow:\s*['"`](?!\$\{)/g;

describe('components/notifications — no hardcoded colors/radii/shadows (gate 1)', () => {
  it('carries no hex color literal', () => {
    const offenders: string[] = [];
    for (const file of sourceFiles(DIR)) {
      if (file === THIS_FILE) continue;
      const text = readFileSync(file, 'utf8');
      const hits = text.match(HEX_COLOR);
      if (hits) offenders.push(`${relative(DIR, file)}: ${hits.join(', ')}`);
    }
    expect(offenders, 'use useTheme().colors — no inline hex').toEqual([]);
  });

  it('carries no rgba()/rgb() literal', () => {
    const offenders: string[] = [];
    for (const file of sourceFiles(DIR)) {
      if (file === THIS_FILE) continue;
      const text = readFileSync(file, 'utf8');
      const hits = text.match(RGBA_LITERAL);
      if (hits) offenders.push(`${relative(DIR, file)}: ${hits.length} hit(s)`);
    }
    expect(offenders, 'use a theme/glass token — no inline rgb()/rgba()').toEqual([]);
  });

  it('carries no hand-written boxShadow string — only token references', () => {
    const offenders: string[] = [];
    for (const file of sourceFiles(DIR)) {
      if (file === THIS_FILE) continue;
      const text = readFileSync(file, 'utf8');
      const lines = text.split('\n');
      lines.forEach((line, i) => {
        if (HARDCODED_BOX_SHADOW.test(line)) offenders.push(`${relative(DIR, file)}:${i + 1}`);
      });
    }
    expect(offenders, 'boxShadow must reference glass.boxShadow / colors.elevation*, not a literal').toEqual([]);
  });

  it('the outer floating surfaces (tray + toast) use radius.glass, not a bare number', () => {
    const offenders: string[] = [];
    for (const file of sourceFiles(DIR)) {
      if (file === THIS_FILE) continue;
      const text = readFileSync(file, 'utf8');
      const lines = text.split('\n');
      lines.forEach((line, i) => {
        // A bare numeric borderRadius anywhere in this directory is the thing
        // radiusScale.test.ts already forbids app-wide; this just double-checks
        // it locally, cheaply, without re-implementing that gate's full scan.
        if (/borderRadius:\s*\d/.test(line)) offenders.push(`${relative(DIR, file)}:${i + 1}  ${line.trim()}`);
      });
    }
    expect(offenders, 'use radius.<step> / concentric() from styles/tokens').toEqual([]);
  });
});
