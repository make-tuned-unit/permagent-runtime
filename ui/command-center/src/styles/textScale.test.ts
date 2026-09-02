/**
 * The type ramp, and the two gates that keep it from being re-typed.
 *
 * This was the worst number in the design audit: 1,372 hand-written
 * `fontSize:` values against an eight-role ramp that already owned five of
 * them. 1,071 of those were literally re-typing a token — `fontSize: 11` next
 * to a `type.micro` that is 11.
 *
 * The reason they were re-typed rather than spread is worth stating, because
 * it is the reason the obvious codemod was the wrong one: `type.micro` carries
 * `fontWeight: 500` and a 14px leading along with its size, so spreading it
 * into a style object that already sets its own weight and `lineHeight: 1.5`
 * changes the rendering. `textSize` is the size half of the same eight roles,
 * so a caller who needs only the size can still name the role, and the
 * migration was provably pixel-for-pixel.
 *
 * Two rules, in the order they matter:
 *
 *   1. A SIZE THE RAMP OWNS is written as `textSize.<role>`, never as the
 *      number. This one is absolute and app-wide — there is no frontier,
 *      because the codemod finished.
 *
 *   2. A SIZE THE RAMP DOES NOT OWN is FROZEN, not banned. 313 remain, 248 of
 *      them 10px — a size below every reference console's floor and below the
 *      ramp's own smallest role. Whether 10px becomes 11 (labels, numbers) or
 *      12 (prose) is a product call about density on a 1280x800 window, and it
 *      is the user's to make (U2 §2.2 / J1), not a codemod's. So the budget below
 *      says how many exist today and refuses to let the number grow. Adding a
 *      `nano: 10` token is the one option explicitly ruled out: it would
 *      ratify solving density by shrinking, which R7 forbids.
 *
 * The remaining 314, by size: 10 x249, 9 x20, 18 x12, 10.5 x8, 22 x5, 12.5 x4,
 * 28 x4, 26 x3, 9.5 x3, 24 x2, 19 x2, 36 x1, 11.5 x1. The half-pixels and the
 * display-ish outliers (18/19/22/24/26/28/36, sitting next to `title` at 20 and
 * `display` at 32) are a by-hand pass, not a codemod — each one is a real
 * visual decision, which is why none of them moved here.
 *
 * TO LOWER A BUDGET: change the number in the same commit that removes the
 * sizes. It only ever moves down.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

import { textSize, type } from './tokens';

const SRC = fileURLToPath(new URL('..', import.meta.url));

/** The ramp defines these; `tokens.ts` is where they are allowed to be numbers. */
const TOKENS_FILE = 'styles/tokens.ts';

/**
 * Off-ramp sizes still in the tree, by top-level directory. A ceiling, not a
 * target: the gate fails when a directory grows one, and the entry is lowered
 * by hand when a directory sheds one. `components/world`'s share is HUD chrome
 * — the scene exemption (U2 §2.4) covers material constants, not DOM text, and
 * no 3D `<Text>` in this app sizes itself in px anyway.
 */
const OFF_RAMP_BUDGET: Record<string, number> = {
  'components/automate': 33,
  'components/awareness': 8,
  'components/brain': 27,
  'components/browser': 5,
  'components/build': 4,
  'components/chat': 9,
  'components/common': 3,
  'components/dashboard': 33,
  'components/finance': 1,
  'components/inbox': 4,
  'components/inspection': 10,
  'components/notifications': 1,
  'components/people': 4,
  'components/projects': 51,
  'components/sessions': 1,
  'components/settings': 18,
  'components/skills': 2,
  'components/tool-results': 2,
  // 6, down from 7: the Orb's 28px teach-word is now `type.display`. The six
  // that remain are all 10px, and the ruling above says that cohort is a
  // product call about density, not a screen lane's to make.
  'components/voice': 6,
  'components/wizard': 6,
  'components/world': 32,
};

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

/** `fontSize: 11` and `fontSize: '11px'`, with the line each sits on. */
export function fontSizeLiterals(source: string): { line: number; px: number }[] {
  const re = /fontSize: (?:'(\d+(?:\.\d+)?)px'|(\d+(?:\.\d+)?))(?![\d.])/g;
  const out: { line: number; px: number }[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(source)) !== null) {
    out.push({ line: source.slice(0, m.index).split('\n').length, px: Number(m[1] ?? m[2]) });
  }
  return out;
}

const RAMP = new Set<number>(Object.values(textSize));

/** Every off-ramp literal in the tree, grouped by its top-level directory. */
function offRampByDir(): Map<string, string[]> {
  const byDir = new Map<string, string[]>();
  for (const file of sourceFiles(SRC)) {
    const rel = relative(SRC, file);
    if (rel === TOKENS_FILE) continue;
    const dir = rel.split('/').slice(0, 2).join('/');
    for (const { line, px } of fontSizeLiterals(readFileSync(file, 'utf8'))) {
      if (RAMP.has(px)) continue;
      const list = byDir.get(dir) ?? [];
      list.push(`${rel}:${line}  ${px}px`);
      byDir.set(dir, list);
    }
  }
  return byDir;
}

describe('type ramp', () => {
  it('exposes the size of every role and invents none', () => {
    expect(textSize).toEqual({
      display: 32, title: 20, heading: 16, body: 14, small: 13, caption: 12, micro: 11,
    });
    // Derived, so it cannot drift: change `type` and this moves with it.
    for (const [role, px] of Object.entries(textSize)) {
      expect(type[role as keyof typeof textSize].fontSize).toBe(px);
    }
    // `label` is a role you spread whole, for its tracking and its uppercase.
    // Its size is `micro`, so it deliberately has no `textSize` entry of its own.
    expect(type.label.fontSize).toBe(textSize.micro);
  });

  it('is never re-typed as the number it already is', () => {
    const offenders: string[] = [];
    for (const file of sourceFiles(SRC)) {
      const rel = relative(SRC, file);
      if (rel === TOKENS_FILE) continue;
      for (const { line, px } of fontSizeLiterals(readFileSync(file, 'utf8'))) {
        if (RAMP.has(px)) offenders.push(`${rel}:${line}  ${px}px`);
      }
    }
    expect(
      offenders,
      'use textSize.<role> from styles/tokens — the ramp already has this size',
    ).toEqual([]);
  });

  it('grows no new size the ramp does not have', () => {
    const over: string[] = [];
    for (const [dir, hits] of offRampByDir()) {
      const budget = OFF_RAMP_BUDGET[dir];
      if (budget === undefined) {
        over.push(`${dir}: ${hits.length} off-ramp sizes, and no budget declared`);
      } else if (hits.length > budget) {
        over.push(`${dir}: declares ${budget}, found ${hits.length}\n    ${hits.join('\n    ')}`);
      }
    }
    expect(
      over,
      'the off-ramp sizes are frozen: pick a textSize role, or say why the '
        + 'budget must rise. 10px in particular is blocked on a product ruling '
        + '(U2 J1) and is not getting a token.',
    ).toEqual([]);
  });

  it('declares no budget for a directory that has none left', () => {
    const live = offRampByDir();
    const stale = Object.keys(OFF_RAMP_BUDGET).filter(d => !live.has(d));
    expect(stale, 'this directory is clean — delete its budget line').toEqual([]);
  });
});
