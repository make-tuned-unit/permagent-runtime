/**
 * Fitness gate for this directory (UI DAG rule 1): zero hardcoded colors,
 * radii, or shadows on the splash / boot screens. Same shape as
 * `components/notifications/tokens.fitness.test.ts` — see that file for the
 * rationale; this is the splash lane's copy of the same gate.
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
/** An `rgb(...)`/`rgba(...)` literal used as a value. Theme colors already
 *  carry rgba strings — this directory should only ever reference them by
 *  name (`colors.cyanWash`, `colors.cyanGlow`, …), never spell one out. */
const RGBA_LITERAL = /rgba?\(\s*\d/gi;
/** A hand-rolled `boxShadow:` value, as opposed to one built from a token. */
const HARDCODED_BOX_SHADOW = /boxShadow:\s*['"`](?!\$\{)/g;
/** A bare numeric `borderRadius:` — the app-wide radius scale already forbids
 *  re-hardcoding a value it has a step for (`radiusScale.test.ts`); this
 *  directory currently has no rounded surfaces at all, and this keeps it that
 *  way rather than letting one regrow unnoticed. */
const HARDCODED_RADIUS = /borderRadius:\s*\d/g;

describe('components/splash — no hardcoded colors/radii/shadows (gate 1)', () => {
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
    expect(offenders, 'use a theme token — no inline rgb()/rgba()').toEqual([]);
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
    expect(offenders, 'boxShadow must reference a token, not a literal').toEqual([]);
  });

  it('carries no bare numeric borderRadius', () => {
    const offenders: string[] = [];
    for (const file of sourceFiles(DIR)) {
      if (file === THIS_FILE) continue;
      const text = readFileSync(file, 'utf8');
      const lines = text.split('\n');
      lines.forEach((line, i) => {
        if (HARDCODED_RADIUS.test(line)) offenders.push(`${relative(DIR, file)}:${i + 1}  ${line.trim()}`);
      });
    }
    expect(offenders, 'use radius.<step> / concentric() from styles/tokens').toEqual([]);
  });

  it('every transition duration used is a motion token (<500ms per D9)', () => {
    // Splash.tsx / BootScreen.tsx build transition strings from `duration.*`
    // template interpolations, not literal ms numbers — this catches a
    // regression back to a hand-typed `800ms`/`400ms` string.
    const LITERAL_MS_TRANSITION = /transition:\s*[`'"][^`'"]*?\d+ms/g;
    const offenders: string[] = [];
    for (const file of sourceFiles(DIR)) {
      if (file === THIS_FILE) continue;
      const text = readFileSync(file, 'utf8');
      const hits = text.match(LITERAL_MS_TRANSITION);
      if (hits) offenders.push(`${relative(DIR, file)}: ${hits.join(' | ')}`);
    }
    expect(offenders, 'transitions must interpolate duration.* tokens, not a literal ms number').toEqual([]);
  });
});
