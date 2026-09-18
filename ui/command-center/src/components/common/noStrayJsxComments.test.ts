import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * A `//` comment in JSX child position is not a comment. It is text.
 *
 * On 2026-09-18 a four-line `//` note was added just inside a provider's
 * children in FinanceView, and it rendered as four lines of source code across
 * the top of the Finance tab in the shipped desktop app. `tsc --noEmit` was
 * clean, every test passed, and the review read fine — because the code IS
 * valid: JSX children are text unless braced. The only thing that catches it
 * is looking at the screen, which is why it survived to a user.
 *
 * This is the cheap guard. It scans for a line whose first non-space character
 * is `//` while the surrounding context is JSX child position, approximated by
 * "the previous non-blank line ends in `>` or `}` and the next non-blank line
 * begins with `<` or `{`". That approximation is deliberately narrow: a false
 * positive here would block a legitimate comment, so it only fires on the
 * exact shape that bit us. Comments above a `return (`, inside props, or in
 * ordinary statement position are untouched.
 */
const SRC = new URL('../..', import.meta.url).pathname;

function tsxFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry.startsWith('.')) continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) tsxFiles(full, out);
    else if (entry.endsWith('.tsx') && !entry.includes('.test.')) out.push(full);
  }
  return out;
}

/** Lines that will render as visible source text in the running app. */
function strayComments(source: string): number[] {
  const lines = source.split('\n');
  const hits: number[] = [];
  const nonBlankBefore = (i: number) => {
    for (let k = i - 1; k >= 0; k -= 1) if (lines[k].trim()) return lines[k].trim();
    return '';
  };
  // Skip the REST OF THE COMMENT RUN when looking ahead. The shape that
  // shipped was four `//` lines in a row: looking only at the next non-blank
  // line found another comment, not the JSX that follows, so the first draft
  // of this guard did not catch the very bug it was written for.
  const nonBlankAfter = (i: number) => {
    for (let k = i + 1; k < lines.length; k += 1) {
      const candidate = lines[k].trim();
      if (!candidate || candidate.startsWith('//')) continue;
      return candidate;
    }
    return '';
  };
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i].trim();
    if (!line.startsWith('//')) continue;
    const before = nonBlankBefore(i);
    const after = nonBlankAfter(i);
    // Only the exact shape that shipped: sandwiched between an opened JSX
    // element and another JSX element or expression.
    // Inside an OPENING TAG a `//` comment is a real comment and renders
    // nothing — ForumPlaceRail has a legitimate one between `aria-hidden={…}`
    // and an `{...spread}`. The tell is that the preceding line is an
    // attribute (`name={…}` / `name="…"` / a spread), not a closed element.
    const precedingIsAttribute = /^[A-Za-z_$][\w$-]*=/.test(before)
      || before.startsWith('{...');
    const inChildPosition = (before.endsWith('>') || before.endsWith('}'))
      && !before.endsWith('=>')
      && !precedingIsAttribute
      && (after.startsWith('<') || after.startsWith('{'));
    // A run of `//` lines: only flag the first, and only if the run as a whole
    // sits in child position.
    const previousWasComment = i > 0 && lines[i - 1].trim().startsWith('//');
    if (inChildPosition && !previousWasComment) hits.push(i + 1);
  }
  return hits;
}

describe('no // comment renders itself into the UI', () => {
  it('finds the shape that shipped, so the guard is not vacuous', () => {
    const shipped = [
      '    <MoneyContext.Provider value={money}>',
      '    // Finance was the one workspace that painted no background,',
      '    // so it showed whatever the shell happened to be.',
      "    <div style={{ display: 'flex' }}>",
    ].join('\n');
    expect(strayComments(shipped)).toEqual([2]);
  });

  it('does not flag a comment between JSX attributes', () => {
    // Real code from ForumPlaceRail: inside an opening tag, `//` is a comment.
    const attrs = [
      '        aria-hidden={!open}',
      '        // `aria-hidden` alone is a promise WebKit will break.',
      "        {...(open ? {} : { inert: '' })}",
      '      >',
    ].join('\n');
    expect(strayComments(attrs)).toEqual([]);
  });

  it('leaves legitimate comments alone', () => {
    const fine = [
      '  // Describes the function below.',
      '  function thing() {',
      '    return (',
      '      // eslint-disable-next-line',
      '      <div />',
      '    );',
      '  }',
    ].join('\n');
    // `return (` does not end in > or }, so the comment before <div /> is safe.
    expect(strayComments(fine)).toEqual([]);
  });

  it('holds across every component in the app', () => {
    const offenders: string[] = [];
    for (const file of tsxFiles(SRC)) {
      const lines = strayComments(readFileSync(file, 'utf8'));
      for (const line of lines) offenders.push(`${file.slice(SRC.length)}:${line}`);
    }
    expect(
      offenders,
      'a `//` comment in JSX child position renders as visible text — use {/* … */}',
    ).toEqual([]);
  });
});
