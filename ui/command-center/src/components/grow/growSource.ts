/**
 * The Grow directory's source, as one string, for the tests that assert against
 * it rather than against the DOM.
 *
 * Four tests (`growLenses`, `analyticsPanelScope`, `growActionTabs`,
 * `growActions.verify`) pin rules that have no rendered symptom when they break
 * — "Actions is the first lens", "the analytics panels are keyed on the project
 * id", "a delta never renders beside an inconclusive verdict" — by reading the
 * source. Before R9 that source was one file. It is now sixteen, so the tests
 * read this instead: same assertions, same strings, a directory rather than a
 * file.
 *
 * FILE_BREAK is load-bearing, not decoration. `growActions.verify.test.ts`
 * extracts a named function's body by slicing from `function NAME(` to the next
 * top-level `function`, and the last function in a file has no next one — so
 * without a terminator its "body" would run on into whatever module was
 * concatenated after it, and an assertion counting `<ActionVerify` in
 * `ActionCard` would silently be counting the neighbours too. The break is a
 * real (dead) top-level function declaration, so it terminates that scan
 * exactly the way a real module boundary should.
 *
 * Test-only. Nothing in the app imports it, and it must not: it reads the
 * filesystem at module scope.
 */

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const DIR = fileURLToPath(new URL('.', import.meta.url));

const FILE_BREAK = '\n\nfunction __growSourceModuleBoundary__() { /* not real code */ }\n\n';

/**
 * The view leads, then everything else alphabetically.
 *
 * Not cosmetic. `analyticsPanelScope.test.ts` asks "where is `<GrowActions`
 * rendered, and is it keyed on the project?" by taking the FIRST occurrence —
 * which has to be the render site, not a doc comment quoting the render site
 * (`GrowActions.tsx` explains its server-side `generating` flag by quoting
 * ``lens === 'actions' && <GrowActions …>``, and sorts before `GrowView.tsx`).
 * Reading from the entry point outward is also what those tests always did,
 * back when the entry point was the only file.
 */
const ENTRY = 'GrowView.tsx';

function sourceFiles(dir: string, out: string[] = []): string[] {
  const entries = readdirSync(dir).sort((a, b) => {
    if (a === ENTRY) return -1;
    if (b === ENTRY) return 1;
    return a.localeCompare(b);
  });
  for (const entry of entries) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) { sourceFiles(full, out); continue; }
    if (!/\.tsx?$/.test(entry)) continue;
    if (/\.(test|spec)\.tsx?$/.test(entry)) continue;
    if (entry === 'growSource.ts') continue;
    out.push(full);
  }
  return out;
}

/** Every non-test source file in `components/grow`, joined. */
export const GROW_SOURCE: string = sourceFiles(DIR)
  .map((f) => readFileSync(f, 'utf8'))
  .join(FILE_BREAK);
