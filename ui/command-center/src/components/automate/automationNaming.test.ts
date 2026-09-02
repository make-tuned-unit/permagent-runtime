/**
 * One screen called the same thing a "recipe" and an "automation". The tab is
 * Automate, the create button says Create automation, the modal is New
 * Automation, the delete says "Delete automation" — and the section header
 * above all of them said Recipes, with empty states to match.
 *
 * The ruling is Automations. "Recipe" survives as the internal type's name
 * (`RecipeCard`, `kind: 'recipe'`, the `recipe` payload the daemon accepts),
 * which is the only place it was ever right.
 *
 * This is the gate, not the fix: the fix is four strings, and it regrows the
 * moment someone writes the fifth. So the pin is on the rendered text — any
 * user-facing "recipe" in Automate's own source fails here.
 */

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const SOURCE = readFileSync(
  fileURLToPath(new URL('./AutomateView.tsx', import.meta.url)),
  'utf8',
);

/**
 * The identifiers and wire fields that legitimately carry the old word. Each
 * is code, never a string a user reads.
 */
const INTERNAL = [
  /RecipeCard/,
  /RecipeDetail/,
  /kind: 'recipe'/,
  /kind === 'recipe'/,
  /const recipe = /,
  /^\s*recipe,\s*$/,
];

/**
 * Comments are prose for the next reader, not interface copy — and several of
 * the ones in this file exist specifically to record why the internal name
 * stays. Block comments span lines, so the state has to be carried.
 */
function codeOnly(source: string): string[] {
  let inBlock = false;
  return source.split('\n').map(line => {
    let out = '';
    let i = 0;
    while (i < line.length) {
      if (inBlock) {
        const end = line.indexOf('*/', i);
        if (end === -1) return out;
        inBlock = false;
        i = end + 2;
        continue;
      }
      if (line.startsWith('//', i)) return out;
      if (line.startsWith('/*', i)) { inBlock = true; i += 2; continue; }
      out += line[i];
      i += 1;
    }
    return out;
  });
}

describe('Automate names one thing one way', () => {
  it('renders no user-facing "recipe"', () => {
    const offenders: string[] = [];
    const lines = SOURCE.split('\n');
    codeOnly(SOURCE).forEach((code, i) => {
      if (!/recipe/i.test(code)) return;
      if (INTERNAL.some(rx => rx.test(code))) return;
      offenders.push(`${i + 1}: ${lines[i].trim()}`);
    });

    expect(offenders, 'say "automation" — the user-facing word is ruled').toEqual([]);
  });

  it('takes the header and the empty states from the shared vocabulary', () => {
    expect(SOURCE).toContain("import { AUTOMATION } from '../../lib/vocabulary'");
    expect(SOURCE).toContain('<Section title={AUTOMATION.title}');
  });
});
