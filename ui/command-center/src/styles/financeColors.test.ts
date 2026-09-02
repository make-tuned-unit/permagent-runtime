/**
 * Finance (R11) — zero hardcoded colors, radii or shadows.
 *
 * Same job as `radiusScale.test.ts` and `backdropFilter.test.ts`, scoped to
 * one screen: `components/finance/**` is a content-only view (D1 — no
 * toolbar, no sidebar, nothing that earns glass), so its whole design-gate-1
 * obligation is "every fill, border and radius comes from a token". This is
 * the gate that keeps that true going forward rather than just once, at
 * review time.
 *
 * Three separate rules, because "zero hardcoded colors" quietly regrows three
 * different ways: a hex string pasted back in, an `rgba(0,0,0,0.4)` typed
 * instead of reached for a fill token, or a bare pixel radius standing in for
 * `radius.*` / `concentric()`.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const FINANCE_DIR = fileURLToPath(new URL('../components/finance', import.meta.url));

/**
 * The one documented exception: a fixed dark ink for legibility on
 * `AGENT_TRIM.financier`'s fixed gold badge fill. `world/shared/palette.ts`
 * is FROZEN and theme-independent (identity trim, never repainted by state or
 * theme), so there is no theme token this could derive from — the same
 * situation `colors.textOnCyan` solves for the flat cyan fill, just with no
 * generic "ink on a bright identity-trim fill" token minted yet. Named once,
 * at module scope, with a comment, in `FinanceView.tsx`'s
 * `FINANCIER_BADGE_INK` — not sprinkled. A real request for a generic token
 * is filed in this lane's PR body (R11 -> A1c/tokens).
 */
const ALLOWED_HEX = new Set(['#3d2e0a']);

function financeFiles(): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) { walk(full); continue; }
      if (!/\.tsx?$/.test(entry)) continue;
      if (/\.(test|spec)\.tsx?$/.test(entry)) continue;
      out.push(full);
    }
  };
  walk(FINANCE_DIR);
  return out;
}

function lines(file: string): string[] {
  return readFileSync(file, 'utf8').split('\n');
}

const REL = (file: string) => `finance/${file.slice(FINANCE_DIR.length + 1)}`;

describe('finance screen — zero hardcoded colors (R11, design gate 1)', () => {
  it('has no hex color literal outside the one documented exception', () => {
    const HEX = /#[0-9a-fA-F]{3,8}\b/g;
    const offenders: string[] = [];
    for (const file of financeFiles()) {
      lines(file).forEach((line, i) => {
        for (const m of line.match(HEX) ?? []) {
          if (!ALLOWED_HEX.has(m)) offenders.push(`${REL(file)}:${i + 1}  ${m}`);
        }
      });
    }
    expect(
      offenders,
      'read the color from useTheme().colors — a hex literal on this screen is either a duplicate of an '
      + 'existing token or a value that belongs in tokens.ts, not inline',
    ).toEqual([]);
  });

  it('has no literal rgba()/rgb() color — fills come from theme tokens, not hand-typed alpha', () => {
    // A LITERAL color: rgba(<digit>... — not a template built from tokens
    // (e.g. `warnFill`'s `rgba(${r},${g},${b},${a})`, which takes a theme
    // color in and is exactly the derivation this rule wants, not the thing
    // it's guarding against).
    const RGBA_LITERAL = /rgba?\(\s*\d/g;
    const offenders: string[] = [];
    for (const file of financeFiles()) {
      lines(file).forEach((line, i) => {
        if (RGBA_LITERAL.test(line)) offenders.push(`${REL(file)}:${i + 1}  ${line.trim()}`);
        RGBA_LITERAL.lastIndex = 0;
      });
    }
    expect(
      offenders,
      'use colors.fillSubtle/fillHover/fillActive (or another theme token) instead of a hand-typed rgba() — '
      + 'a literal alpha value is invisible on the theme it was not tuned against',
    ).toEqual([]);
  });

  it('has no bare pixel border-radius — use radius.* or concentric()', () => {
    // Matches `borderRadius: 10` but not `borderRadius: radius.sm` or
    // `borderRadius: concentric(radius.sm, 2)`.
    const BARE_RADIUS = /borderRadius:\s*['"]?\d/;
    const offenders: string[] = [];
    for (const file of financeFiles()) {
      lines(file).forEach((line, i) => {
        if (BARE_RADIUS.test(line)) offenders.push(`${REL(file)}:${i + 1}  ${line.trim()}`);
      });
    }
    expect(
      offenders,
      'use the radius scale (radius.xs/sm/md/lg/xl/pill) or concentric(outer, padding) for a nested shape',
    ).toEqual([]);
  });

  it('has no boxShadow literal — the ambient shadow ladder lives on colors.elevation*', () => {
    // Flags a boxShadow whose value is a quoted string (a hand-built shadow),
    // not one referencing a colors.* / theme identifier.
    const SHADOW_LITERAL = /boxShadow:\s*['"`]/;
    const offenders: string[] = [];
    for (const file of financeFiles()) {
      lines(file).forEach((line, i) => {
        if (SHADOW_LITERAL.test(line)) offenders.push(`${REL(file)}:${i + 1}  ${line.trim()}`);
      });
    }
    expect(
      offenders,
      'compose boxShadow from colors.elevationRaised/elevationOverlay/elevationFloating/cardHighlight, not a '
      + 'hand-written shadow string',
    ).toEqual([]);
  });

  it('names its one exception with a reason, so the allow-list cannot grow silently', () => {
    expect(ALLOWED_HEX.size, 'this test\'s hex allow-list grew — every entry needs the same kind of comment FinanceView.tsx has for #3d2e0a').toBe(1);
  });
});
